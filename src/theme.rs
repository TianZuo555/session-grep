use catppuccin::PALETTE;
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Frappe,
    Paper,
    Midnight,
    Orbit,
    Akua,
}

#[derive(Clone, Copy)]
pub struct Colors {
    pub base: Color,
    pub text: Color,
    pub mauve: Color,
    pub surface2: Color,
    pub overlay1: Color,
    pub teal: Color,
    pub subtext0: Color,
    pub surface0: Color,
    pub red: Color,
    pub overlay0: Color,
    pub blue: Color,
    pub lavender: Color,
    pub green: Color,
    pub mantle: Color,
    pub sapphire: Color,
    pub yellow: Color,
    pub crust: Color,
    pub badge_text: Color,
    pub peach: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    pub const ALL: [Self; 5] = [
        Self::Frappe,
        Self::Paper,
        Self::Midnight,
        Self::Orbit,
        Self::Akua,
    ];

    pub fn previous(self) -> Self {
        match self {
            Self::Frappe => Self::Akua,
            Self::Paper => Self::Frappe,
            Self::Midnight => Self::Paper,
            Self::Orbit => Self::Midnight,
            Self::Akua => Self::Orbit,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Frappe => Self::Paper,
            Self::Paper => Self::Midnight,
            Self::Midnight => Self::Orbit,
            Self::Orbit => Self::Akua,
            Self::Akua => Self::Frappe,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Frappe => "Frappe",
            Self::Paper => "Paper",
            Self::Midnight => "Midnight",
            Self::Orbit => "Orbit",
            Self::Akua => "Akua",
        }
    }

    pub fn colors(self) -> Colors {
        match self {
            Self::Frappe => {
                let c = PALETTE.frappe.colors;
                let color = |c: catppuccin::Color| Color::Rgb(c.rgb.r, c.rgb.g, c.rgb.b);
                Colors {
                    base: color(c.base),
                    text: color(c.text),
                    mauve: color(c.mauve),
                    surface2: color(c.surface2),
                    overlay1: color(c.overlay1),
                    teal: color(c.teal),
                    subtext0: color(c.subtext0),
                    surface0: color(c.surface0),
                    red: color(c.red),
                    overlay0: color(c.overlay0),
                    blue: color(c.blue),
                    lavender: color(c.lavender),
                    green: color(c.green),
                    mantle: color(c.mantle),
                    sapphire: color(c.sapphire),
                    yellow: color(c.yellow),
                    crust: color(c.crust),
                    badge_text: color(c.crust),
                    peach: color(c.peach),
                }
            }
            // GitHub's default light palette.
            Self::Paper => Colors {
                base: rgb(0xffffff),
                text: rgb(0x1f2328),
                mauve: rgb(0x8250df),
                surface2: rgb(0x8c959f),
                overlay1: rgb(0x59636e),
                teal: rgb(0x1b7c83),
                subtext0: rgb(0x59636e),
                surface0: rgb(0xddf4ff),
                red: rgb(0xcf222e),
                overlay0: rgb(0x6e7781),
                blue: rgb(0x0969da),
                lavender: rgb(0x6639ba),
                green: rgb(0x1a7f37),
                mantle: rgb(0xf6f8fa),
                sapphire: rgb(0x0550ae),
                yellow: rgb(0xffdf5d),
                crust: rgb(0x1f2328),
                badge_text: rgb(0xffffff),
                peach: rgb(0xbc4c00),
            },
            // GitHub's default dark palette.
            Self::Midnight => Colors {
                base: rgb(0x0d1117),
                text: rgb(0xe6edf3),
                mauve: rgb(0xbc8cff),
                surface2: rgb(0x484f58),
                overlay1: rgb(0x9198a1),
                teal: rgb(0x39c5cf),
                subtext0: rgb(0xb1bac4),
                surface0: rgb(0x263c57),
                red: rgb(0xff7b72),
                overlay0: rgb(0x9198a1),
                blue: rgb(0x79c0ff),
                lavender: rgb(0xd2a8ff),
                green: rgb(0x7ee787),
                mantle: rgb(0x161b22),
                sapphire: rgb(0xa5d6ff),
                yellow: rgb(0xe3b341),
                crust: rgb(0x0d1117),
                badge_text: rgb(0x0d1117),
                peach: rgb(0xffa657),
            },
            // One Dark Pro palette.
            Self::Orbit => Colors {
                base: rgb(0x282c34),
                text: rgb(0xabb2bf),
                mauve: rgb(0xc678dd),
                surface2: rgb(0x5c6370),
                overlay1: rgb(0x828997),
                teal: rgb(0x56b6c2),
                subtext0: rgb(0x9da5b4),
                surface0: rgb(0x3e4451),
                red: rgb(0xe06c75),
                overlay0: rgb(0x828997),
                blue: rgb(0x61afef),
                lavender: rgb(0xc678dd),
                green: rgb(0x98c379),
                mantle: rgb(0x21252b),
                sapphire: rgb(0x56b6c2),
                yellow: rgb(0xe5c07b),
                crust: rgb(0x21252b),
                badge_text: rgb(0x21252b),
                peach: rgb(0xd19a66),
            },
            // Dracula palette.
            Self::Akua => Colors {
                base: rgb(0x282a36),
                text: rgb(0xf8f8f2),
                mauve: rgb(0xbd93f9),
                surface2: rgb(0x6272a4),
                overlay1: rgb(0x9da8cc),
                teal: rgb(0x8be9fd),
                subtext0: rgb(0xc7c7cf),
                surface0: rgb(0x44475a),
                red: rgb(0xff5555),
                overlay0: rgb(0x9da8cc),
                blue: rgb(0x8be9fd),
                lavender: rgb(0xff79c6),
                green: rgb(0x50fa7b),
                mantle: rgb(0x21222c),
                sapphire: rgb(0x8be9fd),
                yellow: rgb(0xf1fa8c),
                crust: rgb(0x21222c),
                badge_text: rgb(0x21222c),
                peach: rgb(0xffb86c),
            },
        }
    }
}
