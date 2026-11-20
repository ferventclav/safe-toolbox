//! Version strings as package managers print them: "24.15.0", "7.2.1 (48556)", "< 175.0", "N-125365-g9a01".

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;

static LEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*v?(\d+(?:\.\d+)*)").unwrap());

// Words anywhere ("1.0.0-beta", "5.0-preview.3"), or pip/semver suffixes straight after the numbers
// ("2.0.0rc1", "3.13.0a1", "4.0.0.dev3", "1.0.0-rc.1"). a and b need a number after them, so OpenSSL-style
// stable letters ("1.0.2a") don't match, and neither do build hashes like "N-127043-g5a54".
static PRE_RELEASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(alpha|beta|preview|nightly|canary|insider)|^\s*v?\d+(\.\d+)*[-.]?((a|b)\.?\d+|(rc|pre|dev)\.?\d*)\b").unwrap()
});

/// The leading run of numbers, or None when the string does not start with one.
pub fn parse(version: &str) -> Option<Vec<i64>> {
    if version.trim().is_empty() {
        return None;
    }
    let m = LEADING.captures(version)?;
    m[1].split('.').map(|p| p.parse::<i32>().ok().map(i64::from)).collect()
}

/// winget prints "< 175.0" or "Unknown" when it cannot read the installed version.
pub fn is_unknown(version: &str) -> bool {
    version.trim().is_empty() || version.trim_start().starts_with('<') || version.trim().eq_ignore_ascii_case("Unknown")
}

fn part(v: &[i64], i: usize) -> i64 {
    v.get(i).copied().unwrap_or(0)
}

/// True when the first number changes, or the second for 0.x versions (semver treats those as breaking).
/// Calendar versions such as 2026.10.26 never count, and neither do strings without numbers.
pub fn is_major_jump(current: &str, available: &str) -> bool {
    if is_unknown(current) {
        return false;
    }
    let (Some(a), Some(b)) = (parse(current), parse(available)) else { return false };
    if a[0] >= 1990 || b[0] >= 1990 {
        return false;
    }
    if a[0] != b[0] {
        return true;
    }
    a[0] == 0 && part(&a, 1) != part(&b, 1)
}

pub fn compare(x: &str, y: &str) -> Ordering {
    let a = parse(x).unwrap_or_default();
    let b = parse(y).unwrap_or_default();
    for i in 0..a.len().max(b.len()) {
        let c = part(&a, i).cmp(&part(&b, i));
        if c != Ordering::Equal {
            return c;
        }
    }
    Ordering::Equal
}

/// "24.x" for 24.15.0, "0.13.x" for 0.13.1; what "same family" means in the window.
pub fn family(version: &str) -> String {
    match parse(version) {
        None => String::new(),
        Some(p) if p[0] == 0 && p.len() > 1 => format!("0.{}.x", p[1]),
        Some(p) => format!("{}.x", p[0]),
    }
}

pub fn same_family(a: &str, b: &str) -> bool {
    let f = family(a);
    !f.is_empty() && f == family(b)
}

pub fn is_prerelease(version: &str) -> bool {
    !version.trim().is_empty() && PRE_RELEASE.is_match(version)
}

/// True only when both versions are readable and the "update" is older than what is installed.
pub fn is_downgrade(current: &str, available: &str) -> bool {
    !is_unknown(current) && parse(current).is_some() && parse(available).is_some() && compare(available, current) == Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_leading_numbers() {
        assert_eq!(parse("24.15.0"), Some(vec![24, 15, 0]));
        assert_eq!(parse("v1.2"), Some(vec![1, 2]));
        assert_eq!(parse("7.2.1 (48556)"), Some(vec![7, 2, 1]));
        assert_eq!(parse("N-125365-g9a01"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("99999999999.1"), None);
    }

    #[test]
    fn major_jumps() {
        assert!(is_major_jump("1.2.3", "2.0.0"));
        assert!(!is_major_jump("1.2.3", "1.9.0"));
        assert!(is_major_jump("0.13.1", "0.14.0"));
        assert!(!is_major_jump("2026.10.26", "2027.1.1"));
        assert!(!is_major_jump("< 175.0", "200.0"));
        assert!(!is_major_jump("Unknown", "2.0"));
    }

    #[test]
    fn compare_and_family() {
        assert_eq!(compare("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(family("24.15.0"), "24.x");
        assert_eq!(family("0.13.1"), "0.13.x");
        assert!(same_family("24.1", "24.15.0"));
        assert!(!same_family("abc", "abc"));
    }

    #[test]
    fn prereleases() {
        for v in ["1.0.0-beta", "5.0-preview.3", "2.0.0rc1", "3.13.0a1", "4.0.0.dev3", "1.0.0-rc.1", "2.0 Insider"] {
            assert!(is_prerelease(v), "{v}");
        }
        for v in ["1.0.2a", "N-127043-g5a54", "1.2.3", ""] {
            assert!(!is_prerelease(v), "{v}");
        }
    }

    #[test]
    fn downgrades() {
        assert!(is_downgrade("2.0", "1.9"));
        assert!(!is_downgrade("Unknown", "1.0"));
        assert!(!is_downgrade("1.0", "abc"));
    }
}
