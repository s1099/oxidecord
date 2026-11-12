//! Discord's own sticker packs. Guild stickers come with the gateway's guilds.

use twilight_http::request::{Method, RequestBuilder};

use super::request;
use crate::discord::model::{RawStickerPacks, StickerPack, convert_sticker_packs};

pub async fn fetch_sticker_packs() -> Result<Vec<StickerPack>, String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Get, "sticker-packs".into()).build()?;
        let packs = client
            .request::<RawStickerPacks>(request)
            .await?
            .model()
            .await?;
        Ok(convert_sticker_packs(packs))
    })
    .await
}
