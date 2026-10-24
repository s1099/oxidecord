//! How large a file Discord accepts, which depends on who sends it and where.
//!
//! Nothing in the API reports these limits, so they're the numbers Discord
//! publishes: its File Attachments FAQ for Nitro (as of August 2026, when the
//! free limit went from 10 MB to 20 MB and Nitro's to 1 GB), and its Server
//! Boosting FAQ for boosts. A file may be as large as the higher of the
//! sender's Nitro limit and the guild's boost limit.

use twilight_model::user::PremiumType;

const MIB: u64 = 1024 * 1024;

/// The per-file limit without Nitro or a boosted guild.
pub const DEFAULT_UPLOAD_LIMIT: u64 = 20 * MIB;

/// The most one message's attachments may add up to, whatever the per-file
/// limit. They go up inside the message request, and Cloudflare turns away a
/// larger one before Discord sees it — Discord's own client gets Nitro's full
/// gigabyte through only by uploading to cloud storage first.
pub const MAX_UPLOAD_TOTAL: u64 = 500 * MIB;

/// The per-file limit a tier of Nitro grants anywhere the user sends.
pub(super) fn premium_upload_limit(tier: PremiumType) -> u64 {
    match tier {
        PremiumType::Nitro => 1024 * MIB,
        PremiumType::NitroClassic | PremiumType::NitroBasic => 50 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}

/// The per-file limit a guild's boost level grants everyone sending there.
/// Level 1 doesn't raise it.
pub(in crate::discord) fn boost_upload_limit(premium_tier: u8) -> u64 {
    match premium_tier {
        2 => 50 * MIB,
        3.. => 100 * MIB,
        _ => DEFAULT_UPLOAD_LIMIT,
    }
}
