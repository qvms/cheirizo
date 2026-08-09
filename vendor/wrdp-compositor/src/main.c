// SPDX-License-Identifier: GPL-2.0-only
/* WRDP modifications, 2026. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "common/dir.h"
#include "common/fd-util.h"
#include "common/font.h"
#include "common/mem.h"
#include "common/spawn.h"
#include "config/session.h"
#include "wrdp-compositor.h"
#include "theme.h"
#include "menu/menu.h"

struct rcxml rc = { 0 };

static const struct option long_options[] = {
	{"config", required_argument, NULL, 'c'},
	{"config-dir", required_argument, NULL, 'C'},
	{"debug", no_argument, NULL, 'd'},
	{"exit", no_argument, NULL, 'e'},
	{"help", no_argument, NULL, 'h'},
	{"merge-config", no_argument, NULL, 'm'},
	{"reconfigure", no_argument, NULL, 'r'},
	{"startup", required_argument, NULL, 's'},
	{"session", required_argument, NULL, 'S'},
	{"version", no_argument, NULL, 'v'},
	{"verbose", no_argument, NULL, 'V'},
	{0, 0, 0, 0}
};

static const char wrdp_compositor_usage[] =
"Usage: wrdp-compositor [options...]\n"
"  -c, --config <file>      Specify config file (with path)\n"
"  -C, --config-dir <dir>   Specify config directory\n"
"  -d, --debug              Enable full logging, including debug information\n"
"  -e, --exit               Exit the compositor\n"
"  -h, --help               Show help message and quit\n"
"  -m, --merge-config       Merge user config files/theme in all XDG Base Dirs\n"
"  -r, --reconfigure        Reload the compositor configuration\n"
"  -s, --startup <command>  Run command on startup\n"
"  -S, --session <command>  Run command on startup and terminate on exit\n"
"  -v, --version            Show version number and quit\n"
"  -V, --verbose            Enable more verbose logging\n";

static void
usage(void)
{
	printf("%s", wrdp_compositor_usage);
	exit(0);
}

static void
die_on_detecting_suid(void)
{
	if (geteuid() != 0 && getegid() != 0) {
		return;
	}
	if (getuid() == geteuid() && getgid() == getegid()) {
		return;
	}
	wlr_log(WLR_ERROR, "SUID detected - aborting");
	exit(EXIT_FAILURE);
}

static bool
read_self_start_ticks(char *buffer, size_t size)
{
	FILE *stream = fopen("/proc/self/stat", "r");
	if (!stream) {
		return false;
	}
	char *line = NULL;
	size_t capacity = 0;
	bool success = false;
	if (getline(&line, &capacity, stream) < 0) {
		goto out;
	}
	char *after_comm = strrchr(line, ')');
	if (!after_comm || after_comm[1] != ' ') {
		goto out;
	}
	char *saveptr = NULL;
	char *token = strtok_r(after_comm + 2, " ", &saveptr);
	for (int field = 3; token; field++, token = strtok_r(NULL, " ", &saveptr)) {
		if (field != 22) {
			continue;
		}
		errno = 0;
		char *end = NULL;
		unsigned long long ticks = strtoull(token, &end, 10);
		if (errno != 0 || end == token || (*end != '\0' && *end != '\n')) {
			goto out;
		}
		int written = snprintf(buffer, size, "%llu", ticks);
		success = written > 0 && (size_t)written < size;
		break;
	}
out:
	free(line);
	fclose(stream);
	return success;
}

static void
send_signal_to_wrdp_compositor_pid(int signal)
{
	char *wrdp_compositor_pid = getenv("WRDP_COMPOSITOR_PID");
	if (!wrdp_compositor_pid) {
		wlr_log(WLR_ERROR, "WRDP_COMPOSITOR_PID not set");
		exit(EXIT_FAILURE);
	}
	int pid = atoi(wrdp_compositor_pid);
	if (!pid) {
		wlr_log(WLR_ERROR, "should not send signal to pid 0");
		exit(EXIT_FAILURE);
	}
	kill(pid, signal);
}

struct idle_ctx {
	struct server *server;
	const char *primary_client;
	const char *startup_cmd;
};

static void
idle_callback(void *data)
{
	/* Idle callbacks destroy automatically once triggered */
	struct idle_ctx *ctx = data;

	/* Start session-manager if one is specified by -S|--session */
	if (ctx->primary_client) {
		ctx->server->primary_client_pid = spawn_primary_client(ctx->primary_client);
		if (ctx->server->primary_client_pid < 0) {
			wlr_log(WLR_ERROR, "fatal error starting primary client: %s",
				ctx->primary_client);
			wl_display_terminate(ctx->server->wl_display);
			return;
		}
	}

	session_autostart_init(ctx->server);
	if (ctx->startup_cmd) {
		spawn_async_no_shell(ctx->startup_cmd);
	}
}

int
main(int argc, char *argv[])
{
	char *startup_cmd = NULL;
	char *primary_client = NULL;
	enum wlr_log_importance verbosity = WLR_ERROR;

	int c;
	while (1) {
		int index = 0;
		c = getopt_long(argc, argv, "c:C:dehmrs:S:vV", long_options, &index);
		if (c == -1) {
			break;
		}
		switch (c) {
		case 'c':
			rc.config_file = optarg;
			break;
		case 'C':
			rc.config_dir = optarg;
			break;
		case 'd':
			verbosity = WLR_DEBUG;
			break;
		case 'e':
			send_signal_to_wrdp_compositor_pid(SIGTERM);
			exit(0);
		case 'm':
			rc.merge_config = true;
			break;
		case 'r':
			send_signal_to_wrdp_compositor_pid(SIGHUP);
			exit(0);
		case 's':
			startup_cmd = optarg;
			break;
		case 'S':
			primary_client = optarg;
			break;
		case 'v':
			printf("wrdp-compositor " WRDP_COMPOSITOR_VERSION "\n");
			exit(0);
		case 'V':
			verbosity = WLR_INFO;
			break;
		case 'h':
		default:
			usage();
		}
	}
	if (optind < argc) {
		usage();
	}

	wlr_log_init(verbosity, NULL);

	die_on_detecting_suid();

	session_environment_init();

#if HAVE_NLS
	/* Initialize locale after setting env vars */
	setlocale(LC_ALL, "");
	bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR);
	textdomain(GETTEXT_PACKAGE);
#endif

	rcxml_read(rc.config_file);

	/*
	 * Set environment variable WRDP_COMPOSITOR_PID to the pid of the compositor
	 * so that SIGHUP and SIGTERM can be sent to specific instances using
	 * `kill -s <signal> <pid>` rather than `killall -s <signal> wrdp-compositor`
	 */
	char pid[32];
	snprintf(pid, sizeof(pid), "%d", getpid());
	if (setenv("WRDP_COMPOSITOR_PID", pid, true) < 0) {
		wlr_log_errno(WLR_ERROR, "unable to set WRDP_COMPOSITOR_PID");
		exit(EXIT_FAILURE);
	}
	wlr_log(WLR_DEBUG, "WRDP_COMPOSITOR_PID=%s", pid);

	char start_ticks[32];
	if (!read_self_start_ticks(start_ticks, sizeof(start_ticks))) {
		wlr_log(WLR_ERROR, "unable to read compositor kernel start ticks");
		exit(EXIT_FAILURE);
	}
	if (setenv("WRDP_COMPOSITOR_START_TICKS", start_ticks, true) < 0) {
		wlr_log_errno(WLR_ERROR, "unable to set WRDP_COMPOSITOR_START_TICKS");
		exit(EXIT_FAILURE);
	}
	wlr_log(WLR_DEBUG, "WRDP_COMPOSITOR_START_TICKS=%s", start_ticks);

	/* useful for helper programs */
	if (setenv("WRDP_COMPOSITOR_VER", WRDP_COMPOSITOR_VERSION, true) < 0) {
		wlr_log_errno(WLR_ERROR, "unable to set WRDP_COMPOSITOR_VER");
	} else {
		wlr_log(WLR_DEBUG, "WRDP_COMPOSITOR_VER=%s", WRDP_COMPOSITOR_VERSION);
	}

	if (!getenv("XDG_RUNTIME_DIR")) {
		wlr_log(WLR_ERROR, "XDG_RUNTIME_DIR is unset");
		exit(EXIT_FAILURE);
	}

	increase_nofile_limit();

	struct server server = { 0 };
	server_init(&server);
	server_start(&server);

	struct theme theme = { 0 };
	theme_init(&theme, &server, rc.theme_name);
	rc.theme = &theme;
	server.theme = &theme;

	menu_init(&server);

	/* Delay startup of applications until the event loop is ready */
	struct idle_ctx idle_ctx = {
		.server = &server,
		.primary_client = primary_client,
		.startup_cmd = startup_cmd
	};
	wl_event_loop_add_idle(server.wl_event_loop, idle_callback, &idle_ctx);

	wl_display_run(server.wl_display);

	session_shutdown(&server);

	menu_finish(&server);
	theme_finish(&theme);
	rcxml_finish();
	font_finish();

	server_finish(&server);

	return 0;
}
