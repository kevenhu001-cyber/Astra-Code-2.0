# Headers, primary, and secondary text

- **Headers:** Use `bold`. For markdown with various header levels, leave in the `#` signs.
- **Primary text:** Default.
- **Secondary text:** Use `dim`.

# Brand palette: black, white, orange

Astra Code's brand surface is intentionally a three-color palette. TUI widgets should compose everything from these three colors plus the terminal's default background:

- **Black** is the terminal's default background. We rely on the terminal's default background instead of painting our own; only call sites that need a contrast surface (overlays, banners) may add a custom background.
- **White** is the default foreground. Most content text (assistant messages, user input echoes, neutral chrome) should use the default foreground, so it inherits whatever white/light-grey the user's terminal ships with.
- **Orange** (`255, 165, 0` on dark backgrounds, `180, 90, 0` on light backgrounds) is the Astra accent. See `theme::accent_style_for` for the canonical helper.

Whenever you reach for a new color, prefer one of these three before introducing anything else.

# Foreground colors

- **Default:** Most of the time, just use the default foreground color (white). `reset` can help get it back.
- **User input tips, selection, and status indicators:** Use ANSI `cyan`.
- **Success and additions:** Use ANSI `green`.
- **Errors, failures and deletions:** Use ANSI `red`.
- **Astra brand:** Use the Astra orange accent (RGB `255,165,0`, dark) / `(180,90,0)` on light backgrounds). Apply this to the wordmark, plan / mode / branch / thread / limit indicators, vim mode pill, YOLO mode badge, plugin mention tags, slash command help labels and similar brand surfaces. See `theme::accent_style_for` for the canonical helper.

# Avoid

- Avoid custom colors because there's no guarantee that they'll contrast well or look good in various terminal color themes. (`shimmer.rs` is an exception that works well because we take the default colors and just adjust their levels.) The Astra orange accent is the only exception: it is centralized in `theme.rs` so call sites can use `.fg(Color::Rgb(255, 165, 0))` without drifting.
- Avoid ANSI `black` & `white` as foreground colors because the default terminal theme color will do a better job. (Use `reset` if you need to in order to get those.) The exception is if you need contrast rendering over a manually colored background.
- Avoid ANSI `blue`, `magenta` and `yellow` because for now the style guide doesn't use them. Prefer a foreground color mentioned above.

(There are some rules to try to catch this in `clippy.toml`.)
