//! File size limits. The API doesn't report them, so these are the numbers from
//! Discord's File Attachments FAQ (as of August 2026: free 20 MB, Nitro 1 GB)
//! and Server Boosting FAQ. A file may be as large as the higher of the
//! sender's Nitro limit and the guild's boost limit.

use twilight_model::user::PremiumType;

const MIB: u64 = 1024 * 1024;

pub const DEFAULT_UPLOAD_LIMIT: u64 = 20 * MIB;

/// Cap on one message's attachments combined. They go up inside the message
/// request and Cloudflare rejects a larger one; Discord's client gets Nitro's
/// full gigabyte through only by uploading to cloud storage first.
pub const MAX_UPLOAD_TOTAL: u64 = 500 * MIB;

pub(super) fn premium_upload_limit(tier: PremiumType) -> u64 {
    match tier {
        PremiumType::Nitro => 1024 * MIB,
        PremiumType::NitroClassic | PremiumType::NitroBasic => 50 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}

/// Level 1 doesn't raise the limit.
pub(in crate::discord) fn boost_upload_limit(premium_tier: u8) -> u64 {
    match premium_tier {
        2 => 50 * MIB,
        3.. => 100 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}
