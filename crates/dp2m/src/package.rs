//! `DP2MVxxx.bin` package for the stock updater: header + stock bridge M6M + M5 + final M6M
//!
//! The running firmware flashes the bridge image (segment 1, header 0x70 bit 0), reboots into it, and the
//! bridge flashes the M5 (bit 5, follows segment 1) and the final M6M (bit 4, file offset LE u32 @ 0x64).

use crate::config::Config;
use crate::m5::{S2_LEN, S2_OFF};
use crate::m6m::{C78_LEN, C78_S3_OFF};

const MAGIC: &[u8] = b"SIGMA.CODP2M";
const VERSION_FIELDS: [usize; 3] = [0x10, 0x30, 0x40];
const DATE: usize = 0x50;

pub fn file_name(cfg: &Config) -> Result<String, String> {
    let (major, minor) = cfg.version_pair()?;
    Ok(format!("DP2MV{major}{minor:02}.bin"))
}

pub fn build(stock: &[u8], firm: &[u8], c78: &[u8], cfg: &Config) -> Result<Vec<u8>, String> {
    if stock.len() != C78_S3_OFF + C78_LEN || !stock.starts_with(MAGIC) {
        return Err("stock package is not a DP2M updater image".into());
    }
    if u32::from_le_bytes(stock[0x64..0x68].try_into().unwrap()) as usize != C78_S3_OFF || stock[0x70] & 0x31 != 0x31 {
        return Err("unexpected DP2M package layout (segment offset / stage flags)".into());
    }
    if firm.len() != S2_LEN || c78.len() != C78_LEN {
        return Err("image sizes do not match the package segments".into());
    }
    let mut pkg = Vec::with_capacity(stock.len());
    pkg.extend_from_slice(&stock[..S2_OFF]);
    pkg.extend_from_slice(firm);
    pkg.extend_from_slice(c78);

    let (major, minor) = cfg.version_pair()?;
    let ver = format!("{major}.{minor:02}");
    for off in VERSION_FIELDS {
        pkg[off..off + 16].fill(0);
        pkg[off..off + ver.len()].copy_from_slice(ver.as_bytes());
    }
    pkg[DATE..DATE + 8].copy_from_slice(build_date().as_bytes());
    Ok(pkg)
}

/// `YYYYMMDD` from `SOURCE_DATE_EPOCH` (reproducible builds) or the clock.
fn build_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64));
    // days since 1970-01-01 -> civil date (Hinnant)
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}{m:02}{d:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn epoch_dates() {
        // SAFETY: tests in this module are the only readers of SOURCE_DATE_EPOCH
        unsafe { std::env::set_var("SOURCE_DATE_EPOCH", "1369353600") };
        assert_eq!(super::build_date(), "20130524");
        unsafe { std::env::remove_var("SOURCE_DATE_EPOCH") };
    }
}
