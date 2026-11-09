//! `cargo xtask metainfo [--check]`: write the AppStream file every Linux
//! package installs, from its template and the released versions in
//! CHANGELOG.md, so the release list never drifts from the changelog.
//!
//! Each `## X.Y.Z (YYYY-MM-DD)` heading becomes a `<release>`: its opening
//! paragraph, and the bold titles of its Added items, are the notes. The
//! Unreleased section is left out. Text naming a handle (`@...`) is left out
//! too, since the notes are shown in app stores.

use crate::Result;

const TEMPLATE: &str = "packaging/linux/io.github.Ayatsuri2D.Ayatsuri2D.metainfo.xml.in";
const OUTPUT: &str = "packaging/linux/io.github.Ayatsuri2D.Ayatsuri2D.metainfo.xml";
const CHANGELOG: &str = "CHANGELOG.md";
const RELEASES_URL: &str = "https://github.com/Ayatsuri2D/ayatsuri_2d/releases/tag/v";
/// Added items listed per release; the rest are in the linked notes.
const MAX_ITEMS: usize = 6;

struct Release {
    version: String,
    date: String,
    intro: Vec<String>,
    added: Vec<String>,
}

/// `## 0.6.1 (2026-11-01)` as (version, date).
fn heading(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("## ")?;
    let (version, date) = rest.split_once(" (")?;
    let date = date.strip_suffix(')')?;
    let numeric =
        |s: &str| s.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let date_ok = date.len() == 10
        && date
            .bytes()
            .enumerate()
            .all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    (numeric(version) && date_ok).then(|| (version.to_string(), date.to_string()))
}

/// Markdown to plain text: links keep their words, emphasis and code marks go.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find("](").and_then(|mid| after[mid..].find(')').map(|end| (mid, mid + end))) {
            Some((mid, end)) => {
                out.push_str(&after[..mid]);
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn parse(changelog: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    // where in the current release we are
    let mut section = String::new();
    let mut in_intro = false;
    let mut paragraph: Vec<&str> = Vec::new();
    for line in changelog.lines() {
        if line.starts_with("## ") {
            if in_intro
                && !paragraph.is_empty()
                && let Some(previous) = releases.last_mut()
            {
                previous.intro.push(plain(&paragraph.join(" ")));
            }
            paragraph.clear();
            in_intro = false;
            section.clear();
            if let Some((version, date)) = heading(line) {
                releases.push(Release { version, date, intro: Vec::new(), added: Vec::new() });
                in_intro = true;
            } else {
                // Unreleased, or anything that is not a dated release
                section = "skip".into();
            }
            continue;
        }
        let Some(current) = releases.last_mut() else { continue };
        if section == "skip" {
            continue;
        }
        if let Some(name) = line.strip_prefix("### ") {
            if in_intro && !paragraph.is_empty() {
                current.intro.push(plain(&paragraph.join(" ")));
            }
            paragraph.clear();
            in_intro = false;
            section = name.trim().to_string();
            continue;
        }
        if in_intro {
            if line.trim().is_empty() {
                if !paragraph.is_empty() {
                    current.intro.push(plain(&paragraph.join(" ")));
                    paragraph.clear();
                }
            } else {
                paragraph.push(line.trim());
            }
            continue;
        }
        if section == "Added"
            && let Some(item) = line.strip_prefix("- **")
            && let Some(end) = item.find("**")
        {
            current.added.push(plain(&item[..end]).trim_end_matches(':').to_string());
        }
    }
    releases
}

fn render(releases: &[Release]) -> String {
    let mut out = String::new();
    for r in releases {
        out.push_str(&format!("    <release version=\"{}\" date=\"{}\">\n", r.version, r.date));
        out.push_str(&format!("      <url type=\"details\">{RELEASES_URL}{}</url>\n", r.version));
        let intro: Vec<&String> = r.intro.iter().filter(|p| !p.contains('@')).collect();
        let added: Vec<&String> =
            r.added.iter().filter(|a| !a.contains('@') && !a.is_empty()).take(MAX_ITEMS).collect();
        if !intro.is_empty() || !added.is_empty() {
            out.push_str("      <description>\n");
            for p in intro {
                out.push_str(&format!("        <p>{}</p>\n", escape(p)));
            }
            if !added.is_empty() {
                out.push_str("        <p>New:</p>\n        <ul>\n");
                for a in added {
                    out.push_str(&format!("          <li>{}</li>\n", escape(a)));
                }
                out.push_str("        </ul>\n");
            }
            out.push_str("      </description>\n");
        }
        out.push_str("    </release>\n");
    }
    out.trim_end_matches('\n').to_string()
}

pub fn metainfo(check: bool) -> Result<()> {
    let template = std::fs::read_to_string(TEMPLATE)?;
    let changelog = std::fs::read_to_string(CHANGELOG)?;
    let releases = parse(&changelog);
    if releases.is_empty() {
        return Err(format!("no dated releases found in {CHANGELOG}").into());
    }
    if !template.contains("@RELEASES@") {
        return Err(format!("{TEMPLATE} has no @RELEASES@ line").into());
    }
    let fresh = template.replace("@RELEASES@", &render(&releases));
    // the template's own note is for editors of the template only
    let fresh = match (fresh.find("<!--"), fresh.find("-->")) {
        (Some(a), Some(b)) if a < b && fresh[..a].trim_end().ends_with("?>") => {
            format!(
                "{}<!-- Generated by `cargo xtask metainfo` from {TEMPLATE} and {CHANGELOG}; do not edit by hand. -->{}",
                &fresh[..a],
                &fresh[b + 3..]
            )
        }
        _ => fresh,
    };
    if check {
        let current = std::fs::read_to_string(OUTPUT).unwrap_or_default();
        if current != fresh {
            return Err(format!(
                "{OUTPUT} is out of date with {TEMPLATE} or {CHANGELOG}; run `cargo xtask metainfo`"
            )
            .into());
        }
        println!("metainfo: ok ({} releases)", releases.len());
    } else {
        std::fs::write(OUTPUT, fresh)?;
        println!("metainfo: wrote {} releases to {OUTPUT}", releases.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_releases_and_skips_unreleased_and_handles() {
        let text = "# Changelog\n\n## Unreleased\n\n### Added\n- **Not yet**: x\n\n\
                    ## 0.2.0 (2026-01-02)\n\nIntro with a [link](https://x) and `code`.\n\n\
                    Asked for by @someone.\n\n### Added\n\n- **Thing one** (#1): words\n\
                    - **Thing `two`**: more\n\n### Fixed\n- **Not listed**\n\n\
                    ## 0.1.0 (2026-01-01)\n\n### Added\n- **First**\n";
        let r = parse(text);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].version, "0.2.0");
        assert_eq!(r[0].date, "2026-01-02");
        assert_eq!(r[0].intro[0], "Intro with a link and code.");
        assert_eq!(r[0].added, vec!["Thing one".to_string(), "Thing two".to_string()]);
        let xml = render(&r);
        assert!(!xml.contains("someone"));
        assert!(!xml.contains("Not yet"));
        assert!(!xml.contains("Not listed"));
        assert!(xml.contains("<release version=\"0.1.0\" date=\"2026-01-01\">"));
    }

    #[test]
    fn heading_needs_a_version_and_a_date() {
        assert!(heading("## Unreleased").is_none());
        assert!(heading("## 0.6.1 (2026-11-01)").is_some());
        assert!(heading("## 0.6.1 (soon)").is_none());
    }
}
