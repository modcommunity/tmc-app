use crate::error::{AppError, AppResult};
use crate::plugins::manifest::Theme;

/// Theme plugins: a fixed set of colour tokens, strictly validated.
///
/// The tokens mirror `@modcommunity/shared`'s `theme.css`, so a theme plugin
/// restyles the app the same way the design system does — and can restyle only
/// what the design system exposes.
///
/// The alternative, letting a plugin ship CSS, was rejected for two reasons
/// that are not stylistic:
///
///   * **`url()` is a network request.** A background image or a webfont in
///     plugin CSS would beacon the author's server on every launch, from every
///     user, with no allow-list applying — the CSP would have to permit it for
///     the feature to work at all.
///   * **CSS can hide UI.** `display: none` on the plugin manager, or a
///     transparent overlay on a confirm dialog, turns a cosmetic feature into a
///     way to stop the user removing the plugin or to trick them into
///     approving something. Tokens cannot reposition anything.
const ALLOWED_TOKENS: &[&str] = &[
    "--background",
    "--surface",
    "--surface-secondary",
    "--surface-tertiary",
    "--surface-hover",
    "--header",
    "--footer",
    "--foreground",
    "--muted",
    "--border",
    "--accent",
    "--accent-foreground",
    "--accent-hover",
    "--danger",
    "--danger-foreground",
    "--danger-hover",
    "--success",
    "--success-foreground",
    "--warning",
    "--warning-foreground",
];

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()/rgba()`, `hsl()/hsla()`, `oklch()`.
///
/// Deliberately not "any valid CSS colour": `var(--x)` would let a token
/// reference another token and recurse, and a bare keyword list is long enough
/// that maintaining it is worse than requiring an explicit value.
fn is_valid_color(value: &str) -> bool {
    let v = value.trim();

    if v.is_empty() || v.len() > 64 {
        return false;
    }

    // Anything that could open a new construct. Checked before the shape
    // matching below so a malformed value cannot sneak through a function arm.
    if v.contains(';')
        || v.contains('{')
        || v.contains('}')
        || v.contains("url")
        || v.contains("var")
        || v.contains('\\')
        || v.contains("//")
        || v.contains('@')
    {
        return false;
    }

    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }

    for func in ["rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "oklab("] {
        if let Some(rest) = v.strip_prefix(func) {
            let Some(args) = rest.strip_suffix(')') else {
                return false;
            };

            return !args.is_empty()
                && args.chars().all(|c| {
                    c.is_ascii_digit()
                        || c.is_ascii_whitespace()
                        || matches!(c, '.' | ',' | '%' | '-' | '+' | '/')
                });
        }
    }

    false
}

pub fn validate(theme: &Theme) -> AppResult<()> {
    if theme.label.trim().is_empty() || theme.label.len() > 64 {
        return Err(AppError::invalid("Theme needs a short label."));
    }

    if let Some(base) = &theme.base {
        if base != "light" && base != "dark" {
            return Err(AppError::invalid("Theme base must be 'light' or 'dark'."));
        }
    }

    if theme.tokens.is_empty() || theme.tokens.len() > ALLOWED_TOKENS.len() {
        return Err(AppError::invalid(
            "Theme must set between 1 and all known tokens.",
        ));
    }

    for (token, value) in &theme.tokens {
        if !ALLOWED_TOKENS.contains(&token.as_str()) {
            return Err(AppError::invalid(format!(
                "'{token}' is not a theme token this app supports."
            )));
        }

        if !is_valid_color(value) {
            return Err(AppError::invalid(format!(
                "'{value}' is not an accepted colour for {token}."
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_shapes_are_refused() {
        for bad in [
            "red; background: url(http://evil.test)",
            "url(http://evil.test)",
            "var(--accent)",
            "#12",
            "rgb(0,0,0);}",
            "expression(alert(1))",
            "#ggg",
            "",
        ] {
            assert!(!is_valid_color(bad), "{bad} should be refused");
        }
    }

    #[test]
    fn ordinary_colours_pass() {
        for good in [
            "#fff",
            "#2f61ea",
            "#2f61eaff",
            "rgb(47, 97, 234)",
            "rgba(47,97,234,0.5)",
            "oklch(0.55 0.2 264.7)",
        ] {
            assert!(is_valid_color(good), "{good} should be accepted");
        }
    }
}
