//! Firmware update notification (read-only - no write/DFU is performed).
//!
//! - mouse.fit: `Config/<ui_folder>/FWLINK.txt` → the zip's ETag/Last-Modified/Length is the fingerprint
//! - Sparklink: `getKeyboardFirmware` (board_id/vid/pid + Xs-API-Key) → version comparison
//!
//! Network outages, timeouts, and server errors are all quietly returned as `Err(UpdateError)`
//! and nothing more - the app side may ignore failures.

use devhub_core::model::FirmwareFeed;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
const SPARKLINK_API: &str = "https://api.sparklinkplayjoy.com/api/v1/getKeyboardFirmware";
// hub.xsyd.top: the public key sent by its bundled SDK (used only for firmware lookups).
const SPARKLINK_KEY: &str = "860DE3DB584158536BC89030D8640E29";
const MOUSEFIT_BASE: &str = "https://www.mouse.fit";

/// Result of one check. `fingerprint` is saved for the next comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateStatus {
    /// A new firmware has been published.
    /// The first observation (no baseline) is always false - no notification is shown right away.
    pub new_available: bool,
    /// Display string for the new version, if known.
    pub version: Option<String>,
    /// Destination of the "Open the official page" button.
    pub url: Option<String>,
    /// Fingerprint for detecting changes to the distribution (ETag + date + size, or the version string).
    pub fingerprint: String,
    /// Time of the check (UNIX seconds).
    pub checked_at: u64,
}

/// All failures are this single type - the caller can simply discard this round.
#[derive(Debug)]
pub struct UpdateError(pub String);

/// Check the official distribution source once.
///
/// `last_fp`: the fingerprint saved last time (empty = first time). `current_fw`: the device's current version.
/// On success, always save the fingerprint (the first time establishes the baseline = new_available: false).
pub fn check(
    feed: &FirmwareFeed,
    current_fw: Option<&str>,
    last_fp: &str,
) -> Result<UpdateStatus, UpdateError> {
    let checked_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let mut st = match feed {
        FirmwareFeed::MouseFit { ui_folder } => mousefit(&agent, ui_folder)?,
        FirmwareFeed::Sparklink { vid, pid, board_id } => {
            sparklink(&agent, *vid, *pid, *board_id, current_fw)?
        }
    };
    st.checked_at = checked_at;
    // A fingerprint change counts as new only for lines where "version comparison was not possible" -
    // if the version could be obtained and it is known to be "same/older", do not notify on a mere header change of the distribution.
    st.new_available =
        st.new_available || (st.version.is_none() && is_new(&st.fingerprint, last_fp));
    Ok(st)
}

/// Version comparison in `a.b.c` form - true only when `dist > cur`.
/// If both cannot be parsed as numeric dotted sequences, it cannot be compared → not treated as new.
fn version_newer(dist: &str, cur: &str) -> bool {
    fn parse(s: &str) -> Option<Vec<u32>> {
        s.trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| p.parse::<u32>())
            .collect::<Result<_, _>>()
            .ok()
    }
    let (Some(mut d), Some(mut c)) = (parse(dist), parse(cur)) else {
        return false;
    };
    let n = d.len().max(c.len());
    d.resize(n, 0);
    c.resize(n, 0);
    d > c
}

/// Whether the fingerprint is "new". The first time (last_fp empty) is always false = it only builds the baseline.
/// Also false for responses where no fingerprint can be obtained (such as Sparklink's empty data).
fn is_new(fp: &str, last_fp: &str) -> bool {
    !fp.is_empty() && !last_fp.is_empty() && fp != last_fp
}

/// mouse.fit: use the HTTP headers of the zip that FWLINK.txt points to as the fingerprint.
fn mousefit(agent: &ureq::Agent, ui_folder: &str) -> Result<UpdateStatus, UpdateError> {
    let feed_url = format!("{MOUSEFIT_BASE}/Config/{ui_folder}/FWLINK.txt");
    let body = agent
        .get(&feed_url)
        .call()
        .map_err(|e| UpdateError(format!("FWLINK: {e}")))?
        .into_string()
        .map_err(|e| UpdateError(format!("FWLINK read: {e}")))?;
    let zip = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| UpdateError("FWLINK is empty".into()))?;
    let zip_url = if zip.starts_with("http") {
        zip.to_string()
    } else {
        format!("{MOUSEFIT_BASE}/{}", zip.trim_start_matches('/'))
    };
    // The zip is ~14MB, so HEAD only.
    let head = agent
        .head(&zip_url)
        .call()
        .map_err(|e| UpdateError(format!("zip HEAD: {e}")))?;
    let etag = head.header("ETag").unwrap_or("");
    let lastmod = head.header("Last-Modified").unwrap_or("");
    let len = head.header("Content-Length").unwrap_or("");
    let fingerprint = format!("{etag}|{lastmod}|{len}");
    // Pick a version-like number string from the file name (…/3955/xxx_3955JM.zip → "3955").
    let version = zip_url
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".zip"))
        .map(|f| f.to_string());
    Ok(UpdateStatus {
        new_available: false,
        version,
        url: Some("https://www.mouse.fit/".into()),
        fingerprint,
        checked_at: 0,
    })
}

/// Sparklink: compare the version in getKeyboardFirmware → data[] with the current value.
fn sparklink(
    agent: &ureq::Agent,
    vid: u16,
    pid: u16,
    board_id: Option<u32>,
    current_fw: Option<&str>,
) -> Result<UpdateStatus, UpdateError> {
    let board_id = board_id.ok_or_else(|| UpdateError("no board_id".into()))?;
    let url = format!(
        "{SPARKLINK_API}?board_id={board_id}&vid={}&pid={}",
        fmt_vidpid(vid),
        fmt_vidpid(pid)
    );
    let res = agent
        .get(&url)
        .set("Xs-API-Key", SPARKLINK_KEY)
        .call()
        .map_err(|e| UpdateError(format!("api: {e}")))?;
    let json: serde_json::Value = res
        .into_json()
        .map_err(|e| UpdateError(format!("json: {e}")))?;
    let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    // 415 ("firmware not uploaded") is a normal "nothing published". Any other non-0 code is also conservatively treated as none.
    if code != 0 && code != 200 {
        return Ok(UpdateStatus {
            new_available: false,
            version: None,
            url: Some("https://hub.xsyd.top/".into()),
            fingerprint: String::new(),
            checked_at: 0,
        });
    }
    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    // The key names of the response are not confirmed because nothing is published - pick likely names in order.
    let pick = |keys: &[&str]| -> Option<String> {
        data.iter().find_map(|item| {
            keys.iter().find_map(|k| {
                item.get(*k)
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    })
                    .filter(|s| !s.is_empty())
            })
        })
    };
    let version = pick(&[
        "version",
        "firmwareVersion",
        "fw_version",
        "fwVersion",
        "ver",
    ]);
    let file = pick(&[
        "url",
        "file",
        "fileUrl",
        "firmwareUrl",
        "downloadUrl",
        "path",
    ]);
    // The fingerprint is "version + file URL". Updates are also detected by version comparison.
    let fingerprint = match (&version, &file) {
        (None, None) => String::new(),
        (v, f) => format!(
            "{}|{}",
            v.clone().unwrap_or_default(),
            f.clone().unwrap_or_default()
        ),
    };
    // Only versions that can be compared count as "new" - differing notation, unknown current value, or a distribution that is
    // older are not called new (a version whose order cannot be defined is "unknown").
    let newer = match (&version, current_fw) {
        (Some(v), Some(cur)) => version_newer(v, cur),
        _ => false,
    };
    Ok(UpdateStatus {
        new_available: newer,
        version,
        url: Some("https://hub.xsyd.top/".into()),
        fingerprint,
        checked_at: 0,
    })
}

/// The API has a track record of accepting vid/pid in decimal, and also of accepting 4-digit hex.
/// Here we send 4-digit hex to match the official site's notation.
fn fmt_vidpid(v: u16) -> String {
    format!("{v:04X}")
}

/// Sparklink's axis name database (`getAxisListV3`). The official HUB uses it to look up the axis library's
/// u16 id → name (e.g. "Magnetic White switch"). Failure is just quietly returned as `Err`.
pub fn sparklink_axis_names(
    board_id: u32,
    vid: u16,
    pid: u16,
) -> Result<std::collections::BTreeMap<u16, String>, UpdateError> {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let url = format!(
        "https://api.sparklinkplayjoy.com/api/v1/getAxisListV3?board_id={board_id}&vid={}&pid={}&t={t}",
        fmt_vidpid(vid),
        fmt_vidpid(pid)
    );
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let res = agent
        .get(&url)
        .set("Xs-API-Key", SPARKLINK_KEY)
        .call()
        .map_err(|e| UpdateError(format!("axis api: {e}")))?;
    let json: serde_json::Value = res
        .into_json()
        .map_err(|e| UpdateError(format!("axis json: {e}")))?;
    let mut out = std::collections::BTreeMap::new();
    // data: [{list:[{axis_id, axis_name}, …]}, …]
    for group in json
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default()
    {
        for a in group
            .get("list")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let id = a.get("axis_id").and_then(|v| v.as_u64()).map(|v| v as u16);
            let name = a
                .get("axis_name")
                .or_else(|| a.get("axisName"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let (Some(id), Some(name)) = (id, name) {
                out.entry(id).or_insert(name);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_new_rules() {
        assert!(!is_new("a|b|1", "")); // first time: only the baseline
        assert!(!is_new("", "old")); // no fingerprint obtained
        assert!(!is_new("same", "same"));
        assert!(is_new("a|b|2", "a|b|1")); // change = new
    }

    #[test]
    fn vidpid_hex() {
        assert_eq!(fmt_vidpid(0x1CA6), "1CA6");
        assert_eq!(fmt_vidpid(0x3009), "3009");
    }
}
