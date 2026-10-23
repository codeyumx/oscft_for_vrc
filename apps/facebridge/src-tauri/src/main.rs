// Release builds are GUI-subsystem (no console window); debug builds keep the console for engine diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMA-BUF renderer kills the app at startup on Wayland with NVIDIA drivers ("Error 71 (Protocol
    // error) dispatching to Wayland display"). Fall back to the shared-memory renderer unless the user chose.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: first statement of main, before any other thread exists.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    facebridge_app::run();
}
