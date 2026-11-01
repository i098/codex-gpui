use crate::host::{Host, Request};
use crate::*;
use anyhow::{Context as _, ensure};
use base64::Engine as _;
use gpui::Div;
use theme::ActiveTheme as _;
use ui::OpenMaps;
use ui::screens::maps::{self, Home, Load, MAP_HEIGHT, Page, Pin, Refresh};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenMaps, window, cx| this.navigate(Route::Maps, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let page = cx.new(|_| Page::new());
    cx.subscribe(&page, |this, page, _: &Refresh, cx| {
        this.load_maps(page, cx)
    })
    .detach();
    root.load_maps(page.clone(), cx);
    root.maps_page = Some(page.clone());
    maps::shell(page, cx)
}

/// The original's `maps-home` query: `safeGet("/local/home")`, byte-for-byte the request the
/// original sends (the desktop fetch headers, then the account and language).
fn request(identity: &host::Identity) -> Request {
    Request::Http(json!({
        "url": "/local/home",
        "method": "GET",
        "headers": {
            "x-openai-web-frontend": "codex_webview",
            "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron",
            "OAI-Language": "en-US",
            "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "ChatGPT-Account-ID": identity.account_id,
            "oai-language": "en-US",
        },
        "returnErrorResponse": true,
    }))
}

/// The original draws the `oai-data` Mapbox Standard style with Mapbox GL, at the first place,
/// else `map_center`, else the whole globe.
// shortcut: the Static Images API cannot render Standard-style imports, so this is the classic
// dark/light style as a flat image; draw the vector style once the port has a map renderer.
fn map_url(home: &Home, dark: bool, width: f32) -> String {
    // The original's public Mapbox token (`chatgpt-mapbox` chunk).
    const TOKEN: &str = "pk.eyJ1Ijoib2FpLWRhdGEiLCJhIjoiY20yYW84aTZ1MGh6bzJwcHpsY2R6OWxyeiJ9.V2VlnBphK96gruxcrHtt-Q";
    let style = if dark { "dark-v11" } else { "light-v11" };
    let overlay = home
        .pins
        .iter()
        .map(|pin| format!("pin-s+3b82f6({:.5},{:.5})", pin.longitude, pin.latitude))
        .collect::<Vec<_>>()
        .join(",");
    let camera = match (home.pins.is_empty(), home.center) {
        (false, _) => "auto".to_owned(),
        (
            true,
            Some(Pin {
                latitude,
                longitude,
            }),
        ) => format!("{longitude:.5},{latitude:.5},12"),
        (true, None) => "0,0,0.5".to_owned(),
    };
    let overlay = if overlay.is_empty() {
        String::new()
    } else {
        format!("{overlay}/")
    };
    format!(
        "https://api.mapbox.com/styles/v1/mapbox/{style}/static/{overlay}{camera}/{}x{}@2x\
         ?access_token={TOKEN}&attribution=false&logo=false",
        width as u32, MAP_HEIGHT as u32,
    )
}

/// An absolute image URL through the host, like the plugin icons: the host attaches no
/// ChatGPT credentials to another origin.
pub(super) fn image(host: &dyn Host, url: &str) -> Result<Arc<gpui::Image>> {
    let response = host.fetch(&Request::Http(
        json!({"url": url, "method": "GET", "headers": {}, "retry": "never"}),
    ))?;
    let response = response.get("value").unwrap_or(&response);
    let wire = &response["response"]["__response"];
    let status = wire["status"].as_u64().context("invalid image response")?;
    ensure!((200..300).contains(&status), "image HTTP status {status}");
    let mime = wire["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|header| {
            (header[0].as_str()?.eq_ignore_ascii_case("content-type"))
                .then(|| header[1].as_str())
                .flatten()
        })
        .unwrap_or("image/png");
    let format = gpui::ImageFormat::from_mime_type(mime.split(';').next().unwrap_or(mime))
        .context("unsupported image type")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(
        wire["base64"]
            .as_str()
            .context("image response is not binary")?,
    )?;
    Ok(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

impl Root {
    pub(crate) fn load_maps(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.load = Load::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let identity = host.identity().ok_or_else(|| "not signed in".to_owned())?;
            host.fetch(&request(&identity))
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
                .and_then(|body| maps::home(&body))
        });
        let dark = cx.theme().appearance == theme::Appearance::Dark;
        let host = self.host.clone();
        cx.spawn(async move |_, cx| {
            let result = read.await;
            // A failed read still shows the empty map, like the original's `showEmptyMap`.
            let home = result.clone().unwrap_or_default();
            let current = page.update(cx, |page, cx| {
                let current = page.generation == generation;
                if current {
                    page.load = result.map_or_else(Load::Failed, Load::Ready);
                    cx.notify();
                }
                current
            });
            if !current {
                return;
            }
            let url = map_url(&home, dark, 728.);
            let images = cx
                .background_spawn(async move {
                    let map = image(host.as_ref(), &url).ok();
                    let cards = home
                        .cards
                        .iter()
                        .filter_map(|card| {
                            let image = image(host.as_ref(), card.image_urls.first()?).ok()?;
                            Some((card.id.clone(), image))
                        })
                        .collect();
                    (map, cards)
                })
                .await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    (page.map, page.images) = images;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_url_frames_pins_then_center_then_globe() {
        let pin = Pin {
            latitude: 37.78,
            longitude: -122.4,
        };
        let pins = Home {
            pins: vec![pin],
            ..Home::default()
        };
        assert!(map_url(&pins, true, 728.).contains(
            "/mapbox/dark-v11/static/pin-s+3b82f6(-122.40000,37.78000)/auto/728x400@2x?"
        ));
        let center = Home {
            center: Some(pin),
            ..Home::default()
        };
        assert!(
            map_url(&center, false, 728.).contains("/light-v11/static/-122.40000,37.78000,12/")
        );
        assert!(map_url(&Home::default(), true, 728.).contains("/static/0,0,0.5/"));
    }
}
