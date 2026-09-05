use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct AboutInfo {
    name: String,
    version: String,
    copyright: String,
    license: &'static str,
    notice: &'static str,
}

#[tauri::command]
pub(crate) fn get_about_info<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<AboutInfo, String> {
    Ok(AboutInfo {
        name: app
            .config()
            .product_name
            .clone()
            .unwrap_or_else(|| app.package_info().name.clone()),
        version: app.package_info().version.to_string(),
        copyright: build_copyright()?,
        license: include_str!("../../../../LICENSE"),
        notice: include_str!("../../../../NOTICE"),
    })
}

fn build_copyright() -> Result<String, String> {
    let mut copyright = None;
    for text in [
        env!("LENS_ABOUT_CONFIG"),
        env!("LENS_ABOUT_PLATFORM_CONFIG"),
        env!("LENS_ABOUT_CONFIG_OVERRIDE"),
    ] {
        let config: serde_json::Value =
            serde_json::from_str(text).map_err(|error| error.to_string())?;
        if let Some(value) = config
            .get("bundle")
            .and_then(|bundle| bundle.get("copyright"))
        {
            copyright = value.as_str().map(str::to_owned);
        }
    }
    copyright
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "Missing build copyright".into())
}

// Both entry points create and focus the window on the event loop.
#[tauri::command]
pub(crate) async fn show_about<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(crate::ui::show_about(&handle));
    })
    .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}
