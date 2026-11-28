//! Tenor, the GIF picker's source. Tenor's own v1 API rather than Discord's
//! `/gifs` proxy, so it goes through the plain HTTP client, not twilight.

use serde::de::DeserializeOwned;

use crate::discord::model::{
    Gif, GifCategory, RawTenorCategories, RawTenorResults, convert_tenor_category,
    convert_tenor_gif,
};
use crate::platform::{http, runtime};

const TENOR_KEY: &str = "3Z0688EVWYKH";
const BASE: &str = "https://g.tenor.com/v1";
/// Tenor's most per request.
const PAGE_SIZE: &str = "50";

pub async fn search_gifs(query: String) -> Result<Vec<Gif>, String> {
    let raw: RawTenorResults = get("search", vec![("q", query), limit()]).await?;
    Ok(raw
        .results
        .into_iter()
        .filter_map(convert_tenor_gif)
        .collect())
}

pub async fn trending_gifs() -> Result<Vec<Gif>, String> {
    let raw: RawTenorResults = get("trending", vec![limit()]).await?;
    Ok(raw
        .results
        .into_iter()
        .filter_map(convert_tenor_gif)
        .collect())
}

pub async fn fetch_gif_categories() -> Result<Vec<GifCategory>, String> {
    let raw: RawTenorCategories = get("categories", vec![("type", "featured".into())]).await?;
    Ok(raw.tags.into_iter().map(convert_tenor_category).collect())
}

fn limit() -> (&'static str, String) {
    ("limit", PAGE_SIZE.into())
}

async fn get<T: DeserializeOwned + Send + 'static>(
    endpoint: &'static str,
    mut params: Vec<(&'static str, String)>,
) -> Result<T, String> {
    params.push(("key", TENOR_KEY.into()));
    // What the picker draws; Tenor's default sends a dozen formats per result.
    params.push(("media_filter", "basic".into()));
    params.push(("contentfilter", "medium".into()));
    let result = runtime::run(async move {
        let response = http::client()
            .get(format!("{BASE}/{endpoint}"))
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        anyhow::Ok(serde_json::from_slice::<T>(&response.bytes().await?)?)
    })
    .await;
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(format!("Tenor: {err}")),
        Err(err) => Err(err.to_string()),
    }
}
