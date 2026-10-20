// Thin shell: the only window boots a tiny bundled loader page (`../loader`,
// set as `build.frontendDist`) which probes the remote cowboy UI and then
// redirects to it. That loader exists to survive the iOS Local Network
// permission prompt (the old direct-to-remote load white-screened until the
// user force-quit); see loader/index.html for the full rationale.
//
// The shell exposes two native capabilities to the remote cowboy UI: the opener
// plugin (the loader's "去设置开启" button → iOS app-settings page) and the haptics
// plugin (DetentSheet open-tap → UIImpactFeedbackGenerator). The remote origin
// gains no other IPC; both are scoped in capabilities/.
//
// `mobile_entry_point` is the symbol the generated iOS/Android projects call;
// on desktop `main.rs` calls `run()` directly.
#[cfg(desktop)]
mod links;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Native haptics for the remote cowboy UI's DetentSheet open-tap. Granted to
        // the remote origin in capabilities/remote-haptics.json (the web side calls
        // `plugin:haptics|impact_feedback` via the injected IPC bridge).
        .plugin(tauri_plugin_haptics::init())
        .setup(|app| {
            let config = &app.config().app.windows[0];
            let window = tauri::WebviewWindowBuilder::from_config(app, config)?;
            #[cfg(desktop)]
            let window = links::configure(window, app.handle().clone());
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running cowboy native shell");
}
