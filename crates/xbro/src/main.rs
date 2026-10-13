//! xbro — a small web browser written from scratch in Rust.
//!
//! ```text
//! xbro [URL]                                  open a window (default: about:home)
//! xbro --screenshot out.png [options] URL     render to a PNG without a window
//! xbro --dump-dom URL                         print the parsed DOM tree
//! xbro --dump-layout URL                      print the layout tree
//! ```

mod gui;
mod headless;

use std::process::ExitCode;

/// Parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub mode: Mode,
    /// Address as typed by the user (resolved with `xbro_net::parse_user_input`).
    pub url: Option<String>,
    /// Viewport width in CSS px (headless) / initial window width.
    pub width: u32,
    /// Viewport height in CSS px; for screenshots `0` means "full page".
    pub height: u32,
    /// Device scale factor for screenshots.
    pub scale: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Gui,
    Screenshot { output: String },
    DumpDom,
    DumpLayout,
    Help,
    Version,
}

pub const USAGE: &str = "\
xbro — a small web browser written from scratch in Rust

USAGE:
    xbro [OPTIONS] [URL]

OPTIONS:
    -s, --screenshot <FILE>   Render the page to a PNG file instead of opening a window
        --dump-dom            Print the DOM tree and exit
        --dump-layout         Print the layout tree and exit
    -w, --width <PX>          Viewport width in CSS px (default 1024)
    -H, --height <PX>         Viewport height in CSS px (default 768; 0 = full page for screenshots)
        --scale <FACTOR>      Device pixel ratio for screenshots (default 1)
    -h, --help                Print help
    -V, --version             Print version

URL may be a full URL, a domain (example.com), a local file path, or search terms.
";

pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        mode: Mode::Gui,
        url: None,
        width: 1024,
        height: 768,
        scale: 1.0,
    };
    let mut i = 0;
    let value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("missing value for {name}"))
    };
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => opts.mode = Mode::Help,
            "-V" | "--version" => opts.mode = Mode::Version,
            "-s" | "--screenshot" => {
                opts.mode = Mode::Screenshot {
                    output: value(&mut i, arg)?,
                }
            }
            "--dump-dom" => opts.mode = Mode::DumpDom,
            "--dump-layout" => opts.mode = Mode::DumpLayout,
            "-w" | "--width" => {
                opts.width = value(&mut i, arg)?
                    .parse()
                    .map_err(|_| "invalid --width".to_string())?;
                if opts.width == 0 {
                    return Err("--width must be positive".into());
                }
            }
            "-H" | "--height" => {
                opts.height = value(&mut i, arg)?
                    .parse()
                    .map_err(|_| "invalid --height".to_string())?
            }
            "--scale" => {
                opts.scale = value(&mut i, arg)?
                    .parse()
                    .map_err(|_| "invalid --scale".to_string())?;
                if !(opts.scale > 0.0 && opts.scale <= 8.0) {
                    return Err("--scale must be in (0, 8]".into());
                }
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(format!("unknown option: {arg}"))
            }
            _ => {
                if opts.url.is_some() {
                    return Err(format!("unexpected argument: {arg}"));
                }
                opts.url = Some(arg.to_string());
            }
        }
        i += 1;
    }
    if matches!(
        opts.mode,
        Mode::Screenshot { .. } | Mode::DumpDom | Mode::DumpLayout
    ) && opts.url.is_none()
    {
        return Err("a URL is required".into());
    }
    Ok(opts)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("xbro: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match &opts.mode {
        Mode::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Mode::Version => {
            println!("xbro {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Mode::Gui => gui::run(&opts),
        Mode::Screenshot { .. } | Mode::DumpDom | Mode::DumpLayout => headless::run(&opts),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xbro: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_modes() {
        assert_eq!(parse_args(&args("")).unwrap().mode, Mode::Gui);
        let o = parse_args(&args("-s out.png -w 800 -H 0 --scale 2 example.com")).unwrap();
        assert_eq!(
            o.mode,
            Mode::Screenshot {
                output: "out.png".into()
            }
        );
        assert_eq!((o.width, o.height, o.scale), (800, 0, 2.0));
        assert_eq!(o.url.as_deref(), Some("example.com"));
        assert!(parse_args(&args("--dump-dom")).is_err());
        assert!(parse_args(&args("--bogus x")).is_err());
        assert!(parse_args(&args("a b")).is_err());
    }
}
