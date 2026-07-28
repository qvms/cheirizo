//! Production RDP listener and per-user connection binding.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use ironrdp_pdu::rdp::capability_sets::server_codecs_capabilities;
use ironrdp_server::{
    BoundConnection, ConnectionBinder, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent,
    MouseEvent, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{info, warn};

trait PeerAwareValidator: Send + Sync {
    fn set_peer_ip(&self, ip: std::net::IpAddr);
    fn prune_stale_entries(&self);
}

impl PeerAwareValidator for crate::security::PamValidator {
    fn set_peer_ip(&self, ip: std::net::IpAddr) {
        self.set_peer_ip(ip);
    }
    fn prune_stale_entries(&self) {
        self.prune_stale_entries();
    }
}

impl PeerAwareValidator for crate::security::StaticPasswordValidator {
    fn set_peer_ip(&self, ip: std::net::IpAddr) {
        self.set_peer_ip(ip);
    }
    fn prune_stale_entries(&self) {
        self.prune_stale_entries();
    }
}

use crate::{
    config::Config,
    rdp::channels::clipboard::{
        ClipboardOrchestrator, ClipboardOrchestratorConfig, WrdpCliprdrFactory,
    },
    rdp::server::{DisplayChannelHandler, EgfxChannelFactory, InputChannelHandler},
};

pub async fn run(config: Config) -> Result<()> {
    info!("starting wrdp production single-daemon listener");

    let tls_config = crate::security::TlsConfig::from_files_with_options(
        &config.security.cert_path,
        &config.security.key_path,
        config.security.require_tls_13,
    )
    .context("failed to load TLS certificates")?;
    let tls_acceptor = ironrdp_server::tokio_rustls::TlsAcceptor::from(tls_config.server_config());

    let listen_addr: SocketAddr = config
        .server
        .listen_addr
        .parse()
        .context("invalid server.listen_addr")?;
    // Keep RemoteFX advertised as the compatible bitmap codec baseline. Some
    // Microsoft mobile clients only open the rdpgfx dynamic channel when the
    // server also advertises a graphics codec in the core capability exchange.
    // EGFX/AVC remains the preferred transport once the DVC is negotiated.
    let codecs = server_codecs_capabilities(&["remotefx"])
        .map_err(|e| anyhow::anyhow!("failed to create bitmap codec capabilities: {e}"))?;

    let active_bound_user = Arc::new(Mutex::new(None));
    let active_display = Arc::new(Mutex::new(None));

    let codec_policy = crate::rdp::server::EgfxCodecPolicy::parse(&config.egfx.codec)
        .context("invalid egfx.codec")?;
    let (gfx_factory, gfx_server_handle, gfx_handler_state) =
        if config.egfx.enabled && codec_policy != crate::rdp::server::EgfxCodecPolicy::Bitmap {
            let compression_mode = match config.egfx.zgfx_compression.to_lowercase().as_str() {
                "auto" => ironrdp_graphics::zgfx::CompressionMode::Auto,
                "always" => ironrdp_graphics::zgfx::CompressionMode::Always,
                _ => ironrdp_graphics::zgfx::CompressionMode::Never,
            };
            let gfx_width = std::env::var("WRDP_DEFAULT_WIDTH")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1920);
            let gfx_height = std::env::var("WRDP_DEFAULT_HEIGHT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1080);
            let gfx_factory = EgfxChannelFactory::with_config(
                gfx_width,
                gfx_height,
                false,
                config.egfx.max_frames_in_flight,
                compression_mode,
            )
            .with_codec_policy(codec_policy);
            let gfx_handler_state = gfx_factory.handler_state();
            let gfx_server_handle = gfx_factory.server_handle();
            (
                Some(Box::new(gfx_factory) as Box<dyn ironrdp_server::GfxServerFactory>),
                Some(gfx_server_handle),
                Some(gfx_handler_state),
            )
        } else {
            (None, None, None)
        };

    let clipboard_manager = if config.clipboard.enabled {
        let all_allowed = config.clipboard.allowed_types.is_empty();
        let has_type = |patterns: &[&str]| {
            all_allowed
                || config
                    .clipboard
                    .allowed_types
                    .iter()
                    .any(|t| patterns.iter().any(|p| t.contains(p)))
        };
        let clipboard_config = ClipboardOrchestratorConfig {
            max_data_size: config.clipboard.max_size,
            enable_images: has_type(&["image/"]),
            enable_files: has_type(&["uri-list", "file", "x-special"]),
            enable_html: has_type(&["text/html"]),
            enable_rtf: has_type(&["rtf"]),
            rate_limit_ms: config.clipboard.rate_limit_ms,
            ..ClipboardOrchestratorConfig::default()
        };
        match ClipboardOrchestrator::new(clipboard_config).await {
            Ok(manager) => Some(Arc::new(Mutex::new(manager))),
            Err(e) => {
                warn!(
                    "production clipboard initialization failed, continuing without CLIPRDR: {e:#}"
                );
                None
            }
        }
    } else {
        None
    };
    let cliprdr_factory = clipboard_manager.as_ref().map(|mgr| {
        Box::new(WrdpCliprdrFactory::new(Arc::clone(mgr)))
            as Box<dyn ironrdp_server::CliprdrServerFactory>
    });

    let mut rdp_server = RdpServer::builder()
        .with_addr(listen_addr)
        .with_tls(tls_acceptor)
        .with_no_input()
        .with_display_handler(ProductionPlaceholderDisplay)
        .with_bitmap_codecs(codecs)
        .with_cliprdr_factory(cliprdr_factory)
        .with_gfx_factory(gfx_factory)
        .with_sound_factory(None)
        .build();

    let mut peer_validator: Option<Arc<dyn PeerAwareValidator>> = None;
    if config.security.auth_method == "pam" {
        let validator = Arc::new(crate::security::PamValidator::new_with_allowed_username(
            Some("wrdp".to_string()),
            config.security.allowed_username.clone(),
        ));
        rdp_server.set_credential_validator(Some(validator.clone()));
        peer_validator = Some(validator.clone());
        info!("production PAM credential validator attached (service=wrdp)");
    }

    match config.security.auth_method.as_str() {
        "pam" => {}
        "password" => {
            let validator = Arc::new(crate::security::StaticPasswordValidator::new_hashes(
                config.security.password_credentials.clone(),
            )?);
            rdp_server.set_credential_validator(Some(validator.clone()));
            peer_validator = Some(validator);
            info!("production static password credential validator attached");
        }
        "none" => {
            anyhow::bail!(
                "auth_method=none is not supported by the production wrdp daemon; use pam or password"
            );
        }
        other => {
            anyhow::bail!("unsupported auth_method in production wrdp mode: {other}");
        }
    }

    rdp_server.set_connection_binder(Some(Arc::new(ProductionSesmanBinder {
        active_bound_user: Arc::clone(&active_bound_user),
        active_display: Arc::clone(&active_display),
        config: Arc::new(config.clone()),
        server_event_tx: rdp_server.event_sender().clone(),
        clipboard_manager: clipboard_manager.clone(),
        gfx_server_handle,
        gfx_handler_state,
    })));

    let listener = production_listener(listen_addr)?;
    info!(
        "wrdp production listener ready on {}",
        listener.local_addr()?
    );

    loop {
        let (stream, peer) = listener
            .accept()
            .await
            .context("failed to accept RDP client")?;
        info!("accepted RDP client connection");
        if let Some(validator) = peer_validator.as_ref() {
            validator.set_peer_ip(peer.ip());
            validator.prune_stale_entries();
        }
        let result = rdp_server.run_connection(stream).await;
        if let Err(e) = &result {
            warn!("RDP client connection ended with error: {e:#}");
        } else {
            info!("RDP client connection ended cleanly");
        }

        if let Some(display) = active_display.lock().await.take() {
            display.release_connection_resources().await;
        }
        if let Some(manager) = clipboard_manager.as_ref() {
            manager.lock().await.clear_connection_state().await;
        }
        if let Some(user) = active_bound_user.lock().await.take() {
            record_production_disconnect(user).await;
        }
    }
}

fn production_listener(listen_addr: SocketAddr) -> Result<tokio::net::TcpListener> {
    if let Some(listener) = adopt_systemd_listener(listen_addr)? {
        return Ok(listener);
    }
    let std_listener = std::net::TcpListener::bind(listen_addr)
        .with_context(|| format!("failed to bind {listen_addr}"))?;
    std_listener
        .set_nonblocking(true)
        .context("failed to set listener nonblocking")?;
    tokio::net::TcpListener::from_std(std_listener).context("failed to create tokio listener")
}

fn adopt_systemd_listener(expected_addr: SocketAddr) -> Result<Option<tokio::net::TcpListener>> {
    let mut listenfd = listenfd::ListenFd::from_env();
    let Some(std_listener) = listenfd
        .take_tcp_listener(0)
        .context("failed to inspect systemd socket activation fd")?
    else {
        return Ok(None);
    };

    std_listener
        .set_nonblocking(true)
        .context("failed to set systemd listener nonblocking")?;
    let actual_addr = std_listener
        .local_addr()
        .context("systemd fd is not a TCP listener")?;
    if actual_addr.port() != expected_addr.port() {
        warn!(
            "systemd listener is {}, config listen_addr is {}; using systemd listener",
            actual_addr, expected_addr
        );
    } else {
        info!("adopted systemd listener {}", actual_addr);
    }
    tokio::net::TcpListener::from_std(std_listener)
        .map(Some)
        .context("failed to adopt systemd listener")
}

async fn record_production_disconnect(user: String) {
    match tokio::task::spawn_blocking(move || {
        let config = crate::sesman::SesmanConfig::for_user(&user)?;
        let idle_timeout_ms = config.idle_timeout_ms;
        let manager = crate::sesman::SessionManager::new(config);
        let status = manager.unbind_single_client()?;
        let stopped_idle = manager.cleanup_idle()?;
        Ok::<_, anyhow::Error>((user, status, stopped_idle, idle_timeout_ms))
    })
    .await
    {
        Ok(Ok((user, status, stopped_idle, idle_timeout_ms))) => {
            info!(
                "recorded wrdp sesman disconnect for user '{}' (health={:?}, stopped_idle={})",
                user, status.health, stopped_idle
            );
            if !stopped_idle && idle_timeout_ms > 0 {
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(idle_timeout_ms)).await;
                    let cleanup_user = user.clone();
                    match tokio::task::spawn_blocking(move || {
                        let config = crate::sesman::SesmanConfig::for_user(&cleanup_user)?;
                        crate::sesman::SessionManager::new(config).cleanup_idle()
                    })
                    .await
                    {
                        Ok(Ok(true)) => info!("stopped idle wrdp user session"),
                        Ok(Ok(false)) => {}
                        Ok(Err(e)) => warn!("failed delayed idle user-session cleanup: {e:#}"),
                        Err(e) => {
                            warn!("delayed idle user-session cleanup task panicked: {e:#}")
                        }
                    }
                });
            }
        }
        Ok(Err(e)) => warn!("failed to record wrdp sesman disconnect: {e:#}"),
        Err(e) => warn!("wrdp sesman disconnect task panicked: {e:#}"),
    }
}

struct ProductionSesmanBinder {
    active_bound_user: Arc<Mutex<Option<String>>>,
    active_display: Arc<Mutex<Option<Arc<DisplayChannelHandler>>>>,
    config: Arc<Config>,
    server_event_tx: tokio::sync::mpsc::UnboundedSender<ironrdp_server::ServerEvent>,
    clipboard_manager: Option<Arc<Mutex<ClipboardOrchestrator>>>,
    gfx_server_handle: Option<Arc<RwLock<Option<ironrdp_server::GfxServerHandle>>>>,
    gfx_handler_state: Option<Arc<RwLock<Option<crate::rdp::server::HandlerState>>>>,
}

#[async_trait::async_trait]
impl ConnectionBinder for ProductionSesmanBinder {
    async fn bind_connection(&self, credentials: &Credentials) -> Result<BoundConnection> {
        let username = normalize_local_account_name(&credentials.username);
        let active_bound_user = Arc::clone(&self.active_bound_user);
        let active_display = Arc::clone(&self.active_display);
        let config = Arc::clone(&self.config);
        let server_event_tx = self.server_event_tx.clone();
        let clipboard_manager = self.clipboard_manager.clone();
        let gfx_server_handle = self.gfx_server_handle.clone();
        let gfx_handler_state = self.gfx_handler_state.clone();

        let ensured_user = username.clone();
        let ensure_result = tokio::task::spawn_blocking(move || {
            let config = crate::sesman::SesmanConfig::for_user(&ensured_user)?;
            let manager = crate::sesman::SessionManager::new(config);
            manager.ensure(crate::sesman::EnsureOptions {
                force_restart: false,
                requested_size: None,
                client_peer: None,
                client_connected: false,
            })
        })
        .await
        .context("sesman ensure task panicked")??;

        let state = ensure_result
            .status
            .state
            .as_ref()
            .context("sesman ensure completed without persisted session state")?;
        let (bound, display) = match build_production_portal_generic_connection(
            &username,
            state.xdg_runtime_dir.join("wayland-0"),
            config,
            server_event_tx,
            clipboard_manager,
            gfx_server_handle,
            gfx_handler_state,
        )
        .await
        {
            Ok(bound) => bound,
            Err(error) => {
                record_production_disconnect(username.clone()).await;
                return Err(error).with_context(|| {
                    format!("failed to bind production RDP handlers for user '{username}'")
                });
            }
        };

        let bound_user = username.clone();
        let bind_result = tokio::task::spawn_blocking(move || {
            let config = crate::sesman::SesmanConfig::for_user(&bound_user)?;
            crate::sesman::SessionManager::new(config).bind_single_client()
        })
        .await;
        match bind_result {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                record_production_disconnect(username.clone()).await;
                return Err(error).context("failed to record bound production client");
            }
            Err(error) => {
                record_production_disconnect(username.clone()).await;
                return Err(error).context("sesman client-bind task panicked");
            }
        }

        *active_bound_user.lock().await = Some(username.clone());
        *active_display.lock().await = Some(display);
        info!(
            "post-auth production sesman bind for user '{}' complete (reused_existing={}, health={:?})",
            username, ensure_result.reused_existing, ensure_result.status.health
        );

        Ok(bound)
    }
}

async fn build_production_portal_generic_connection(
    username: &str,
    wayland_socket: PathBuf,
    config: Arc<Config>,
    server_event_tx: tokio::sync::mpsc::UnboundedSender<ironrdp_server::ServerEvent>,
    clipboard_manager: Option<Arc<Mutex<ClipboardOrchestrator>>>,
    gfx_server_handle: Option<Arc<RwLock<Option<ironrdp_server::GfxServerHandle>>>>,
    gfx_handler_state: Option<Arc<RwLock<Option<crate::rdp::server::HandlerState>>>>,
) -> Result<(BoundConnection, Arc<DisplayChannelHandler>)> {
    #[cfg(not(feature = "portal-generic"))]
    {
        let _ = (
            username,
            wayland_socket,
            config,
            server_event_tx,
            clipboard_manager,
            gfx_server_handle,
            gfx_handler_state,
        );
        anyhow::bail!("production wrdp requires the portal-generic feature");
    }

    #[cfg(feature = "portal-generic")]
    {
        if !wayland_socket.exists() {
            anyhow::bail!(
                "sesman reported a healthy session for '{username}', but Wayland socket {} is missing",
                wayland_socket.display()
            );
        }

        let portal_settings = crate::config::PortalStartupSettings::from_process(&config)?;
        let session_handle =
            crate::rdp::session::backends::PortalSessionBackend::create_session_for_wayland_socket(
                wayland_socket.clone(),
                portal_settings,
            )
            .await
            .with_context(|| {
                format!(
                    "failed to attach portal-generic to {}",
                    wayland_socket.display()
                )
            })?;

        let streams = session_handle.streams();
        if streams.is_empty() {
            anyhow::bail!("portal-generic produced no capture streams for '{username}'");
        }

        let stream_info: Vec<crate::portal::StreamInfo> = streams
            .iter()
            .map(|s| crate::portal::StreamInfo {
                node_id: s.node_id,
                position: (s.position_x, s.position_y),
                size: (s.width, s.height),
                source_type: crate::portal::SourceType::Monitor,
            })
            .collect();
        let initial_size = stream_info
            .first()
            .map_or((1920, 1080), |s| (s.size.0 as u16, s.size.1 as u16));

        let raw_rx = session_handle.direct_frame_receiver()?;

        let display_handler = Arc::new(
            DisplayChannelHandler::new_direct(
                initial_size.0,
                initial_size.1,
                raw_rx,
                stream_info.clone(),
                Some(wayland_socket.clone()),
                None,
                gfx_server_handle,
                gfx_handler_state,
                Arc::clone(&config),
                production_service_registry(),
            )
            .await
            .context("failed to create production display handler")?,
        );
        display_handler
            .set_server_event_sender(server_event_tx)
            .await;

        let primary_stream_id = stream_info.first().map_or(0, |s| s.node_id);
        let min_stream_x = stream_info.iter().map(|s| s.position.0).min().unwrap_or(0);
        let min_stream_y = stream_info.iter().map(|s| s.position.1).min().unwrap_or(0);
        let monitors: Vec<crate::rdp::channels::input::MonitorInfo> = stream_info
            .iter()
            .enumerate()
            .map(|(idx, stream)| crate::rdp::channels::input::MonitorInfo {
                id: idx as u32,
                name: format!("{} monitor {idx}", username),
                x: stream.position.0,
                y: stream.position.1,
                width: stream.size.0,
                height: stream.size.1,
                dpi: 96.0,
                scale_factor: 1.0,
                stream_x: (i64::from(stream.position.0) - i64::from(min_stream_x)) as u32,
                stream_y: (i64::from(stream.position.1) - i64::from(min_stream_y)) as u32,
                stream_width: stream.size.0,
                stream_height: stream.size.1,
                is_primary: idx == 0,
            })
            .collect();

        let clipboard_provider: Option<
            Arc<dyn crate::rdp::channels::clipboard::ClipboardProvider>,
        > = match clipboard_manager.as_ref() {
            Some(manager) => {
                #[cfg(feature = "portal-generic")]
                {
                    match session_handle.clipboard_source() {
                        crate::rdp::session::backend::ClipboardSource::DataControl(ref backend) => {
                            let provider = Arc::new(
                                    crate::rdp::channels::clipboard::providers::DataControlClipboardProvider::new(
                                        Arc::clone(backend),
                                    ),
                                );
                            manager
                                .lock()
                                .await
                                .set_clipboard_provider(provider.clone())
                                .await;
                            display_handler
                                .set_clipboard_manager(Arc::clone(manager))
                                .await;
                            info!(
                                "production portal-generic clipboard provider attached for '{username}'"
                            );
                            Some(provider)
                        }
                        _ => {
                            warn!(
                                "production portal-generic session for '{username}' did not expose data-control clipboard"
                            );
                            None
                        }
                    }
                }
                #[cfg(not(feature = "portal-generic"))]
                {
                    let _ = manager;
                    None
                }
            }
            None => None,
        };

        let (input_tx, input_rx) = tokio::sync::mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let input_handler = InputChannelHandler::new(
            session_handle.clone(),
            monitors,
            primary_stream_id,
            input_tx,
            Some(display_handler.get_update_sender()),
            Some(display_handler.get_gfx_handler_state()),
            input_rx,
            shutdown_rx,
            config.input.cjk_paste_fallback,
            clipboard_provider,
        )
        .context("failed to create production input handler")?;
        display_handler
            .set_input_handler(Arc::new(input_handler.clone()))
            .await;
        Arc::clone(&display_handler).start_pipeline();

        info!(
            "production wrdp attached user '{}' to {} with {} stream(s), initial={}x{}",
            username,
            wayland_socket.display(),
            stream_info.len(),
            initial_size.0,
            initial_size.1,
        );

        let bound = BoundConnection {
            display: Box::new((*display_handler).clone()),
            input: Box::new(ProductionBoundInput {
                inner: input_handler,
                _shutdown_tx: shutdown_tx,
            }),
        };
        Ok((bound, display_handler))
    }
}

fn production_service_registry() -> Arc<crate::services::RuntimeCapabilities> {
    let capabilities = crate::desktop::compositor::CompositorCapabilities::new(
        crate::desktop::compositor::CompositorType::Wlroots {
            name: "wrdp-compositor".to_string(),
        },
        crate::desktop::compositor::PortalCapabilities::default(),
        Vec::new(),
    );
    Arc::new(crate::services::RuntimeCapabilities::from_compositor(
        &capabilities,
    ))
}

struct ProductionBoundInput {
    inner: InputChannelHandler,
    _shutdown_tx: broadcast::Sender<()>,
}

impl RdpServerInputHandler for ProductionBoundInput {
    fn keyboard(&mut self, event: KeyboardEvent) {
        self.inner.keyboard(event);
    }

    fn mouse(&mut self, event: MouseEvent) {
        self.inner.mouse(event);
    }
}

fn normalize_local_account_name(username: &str) -> String {
    let without_domain = username
        .rsplit_once('\\')
        .map_or(username, |(_, user)| user);
    without_domain
        .split_once('@')
        .map_or(without_domain, |(user, _)| user)
        .to_string()
}

#[derive(Clone)]
struct ProductionPlaceholderDisplay;

#[async_trait]
impl RdpServerDisplay for ProductionPlaceholderDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1920,
            height: 1080,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(ProductionNoopUpdates))
    }
}

struct ProductionNoopUpdates;

#[async_trait]
impl RdpServerDisplayUpdates for ProductionNoopUpdates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        std::future::pending::<()>().await;
        unreachable!()
    }
}
