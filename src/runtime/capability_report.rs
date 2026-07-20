//! Operator-facing capability and diagnostic reports.

use crate::{
    desktop::compositor::{BufferType, detect_os_release, probe_capabilities},
    rdp::session::detect_deployment_context,
    services::RuntimeCapabilities,
};
use anyhow::Result;
use serde_json::json;

pub async fn print_capabilities(json_output: bool) -> Result<()> {
    let caps = probe_capabilities().await?;
    let deployment = detect_deployment_context();
    let runtime = RuntimeCapabilities::from_compositor(&caps);
    if json_output {
        let os = detect_os_release();
        let report = json!({"system":{"compositor":caps.compositor.to_string(),"distribution":os.map(|v|v.pretty_name)},"portal":{"version":caps.portal.version,"screencast":caps.portal.supports_screencast},"deployment":deployment.to_string(),"protocols":caps.wayland_globals.iter().map(|g|json!({"name":g.interface,"version":g.version})).collect::<Vec<_>>(),"runtime":{"damage_hints":runtime.damage_hints,"explicit_sync":runtime.explicit_sync,"dmabuf":runtime.dmabuf,"native_input":runtime.native_input,"data_control":runtime.data_control},"recommended":{"capture":format!("{:?}",caps.profile.recommended_capture),"buffer":format!("{:?}",caps.profile.recommended_buffer_type),"codec":if matches!(caps.profile.recommended_buffer_type,BufferType::DmaBuf){"avc420"}else{"bitmap"}}});
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Compositor: {}", caps.compositor);
        println!(
            "Portal ScreenCast: {} (v{})",
            caps.portal.supports_screencast, caps.portal.version
        );
        println!("Deployment: {deployment}");
        println!("Observed Wayland globals: {}", caps.wayland_globals.len());
        println!("Native input: {}", runtime.native_input);
        println!("Data control: {}", runtime.data_control);
    }
    Ok(())
}

pub async fn print_diagnostics() -> Result<()> {
    println!(
        "Wayland display: {}",
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "unavailable".into())
    );
    println!(
        "Session D-Bus: {}",
        zbus::Connection::session().await.is_ok()
    );
    match probe_capabilities().await {
        Ok(caps) => {
            println!("Compositor: {}", caps.compositor);
            println!("ScreenCast portal: {}", caps.portal.supports_screencast);
            println!("Wayland globals: {}", caps.wayland_globals.len());
        }
        Err(error) => println!("Desktop probe failed: {error}"),
    }
    println!("Deployment: {}", detect_deployment_context());
    Ok(())
}
