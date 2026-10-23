//! Contrast gate: WCAG 2 ratios for every colour pair the SDK's own styles create, in light and dark, for each elementary accent.
//! The theme supplies the real colours at run time; the reference values in `tokens.json` (`colors`) stand in for them so a bad
//! pair is caught before anyone looks at a screenshot. Text must reach 4.5:1.

use serde_json::Value;

use crate::tokens::tokens;

#[derive(Debug, Clone)]
pub struct Finding {
    pub pair: String,
    pub theme: &'static str,
    pub accent: Option<String>,
    pub ratio: f64,
    pub needed: f64,
}

impl Finding {
    pub fn ok(&self) -> bool {
        self.ratio >= self.needed
    }
}

type Rgb = [f64; 3];

fn hex(s: &str) -> Rgb {
    let h = s.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
    [c(0), c(2), c(4)]
}

fn luminance(c: Rgb) -> f64 {
    let lin = |v: f64| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `top` at `alpha` over `under`.
fn over(top: Rgb, alpha: f64, under: Rgb) -> Rgb {
    [0, 1, 2].map(|i| top[i] * alpha + under[i] * (1.0 - alpha))
}

pub fn audit() -> Vec<Finding> {
    let t = tokens();
    let mut out = Vec::new();
    let opacity = |k: &str| t["opacity"][k].as_f64().unwrap_or(1.0);
    for theme in ["light", "dark"] {
        let col = |k: &str| -> Rgb { hex(t["colors"][theme][k].as_str().unwrap_or("#000000")) };
        let (bg, fg, err) = (col("bg"), col("fg"), col("error"));
        let mut add = |pair: &str, accent: Option<&str>, r: f64, needed: f64| {
            out.push(Finding {
                pair: pair.into(),
                theme: if theme == "light" { "light" } else { "dark" },
                accent: accent.map(String::from),
                ratio: r,
                needed,
            });
        };
        add("body text on window", None, ratio(fg, bg), 4.5);
        for (name, key) in [("caption text", "caption"), ("muted text", "muted"), ("tool row text", "tool")] {
            add(name, None, ratio(over(fg, opacity(key), bg), bg), 4.5);
        }
        add("error text on window", None, ratio(err, bg), 4.5);
        // The badge uses the window background as its text colour on the error colour.
        add("badge text on error", None, ratio(bg, err), 4.5);
        let alpha = t["colors"]["bubble_alpha"].as_f64().unwrap_or(0.22);
        for (name, v) in t["colors"]["accents"].as_object().into_iter().flatten() {
            let accent = hex(v.as_str().unwrap_or("#000000"));
            let bubble = over(accent, alpha, bg);
            add("user bubble text", Some(name), ratio(fg, bubble), 4.5);
        }
    }
    out
}

pub fn failures() -> Vec<Finding> {
    audit().into_iter().filter(|f| !f.ok()).collect()
}

pub fn to_json(findings: &[Finding]) -> Value {
    findings
        .iter()
        .map(|f| serde_json::json!({ "pair": f.pair, "theme": f.theme, "accent": f.accent, "ratio": (f.ratio * 100.0).round() / 100.0, "needed": f.needed, "ok": f.ok() }))
        .collect::<Vec<_>>()
        .into()
}
