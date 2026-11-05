//! monet in the browser: the same Rust engine, compiled to WebAssembly, drawing
//! with WebGPU (or WebGL2 where WebGPU isn't available yet).
//!
//! The page (web/index.html) has a `<canvas id="monet">`; `start` runs the editor
//! on it. A link like `…/#s=<base64url>` opens a shared sketch.

use wasm_bindgen::JsCast;

/// Run the editor on the page's canvas.
pub fn start() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        let Some(canvas) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("monet"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return fail("the page has no <canvas id=\"monet\">");
        };
        match crate::app::run_web(canvas).await {
            Ok(()) => set_status(None),
            Err(e) => fail(&e),
        }
    });
}

/// Show (or hide, with `None`) the page's loading/status line.
fn set_status(text: Option<&str>) {
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("status"))
    else {
        return;
    };
    match text {
        Some(t) => {
            el.set_text_content(Some(t));
            let _ = el.set_attribute("class", "shown");
        }
        None => {
            let _ = el.set_attribute("class", "");
        }
    }
}

fn fail(e: &str) {
    web_sys::console::error_1(&e.into());
    set_status(Some(&format!(
        "monet couldn't start: {e}. It needs a browser with WebGPU or WebGL2 (recent Chrome, Edge, Safari or Firefox)."
    )));
}

// ── sharing: the sketch in the link ──────────────────────────────────────────

/// Does the page's query string have this flag (`?gl`, `?a&gl`)?
pub fn query_has(flag: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.trim_start_matches('?').split('&').any(|kv| kv.split('=').next() == Some(flag)))
}

/// A link to this page that opens `src`.
pub fn share_url(src: &str) -> String {
    let base = web_sys::window()
        .and_then(|w| w.location().href().ok())
        .map(|h| h.split('#').next().unwrap_or_default().to_string())
        .unwrap_or_default();
    format!("{base}#s={}", crate::share::encode(src.as_bytes()))
}

/// The sketch in this page's link, if it has one.
pub fn shared_sketch() -> Option<String> {
    let hash = web_sys::window()?.location().hash().ok()?;
    let code = hash.trim_start_matches('#').strip_prefix("s=")?;
    String::from_utf8(crate::share::decode(code)?).ok().filter(|s| !s.trim().is_empty())
}

/// Hand `bytes` to the browser as a download called `name`.
pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
    let err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(err)?
        .dyn_into()
        .map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}
