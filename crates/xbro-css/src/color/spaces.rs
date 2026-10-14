//! Color space conversions to sRGB (CSS Color 4 §10 and its sample code).
//!
//! All functions work on `f64` components; sRGB results are in 0..=1 and
//! may fall outside that range (callers clamp).

type Vec3 = [f64; 3];
type Mat3 = [[f64; 3]; 3];

fn mul(m: &Mat3, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// XYZ (D65) to linear sRGB.
const XYZ_TO_LINEAR_SRGB: Mat3 = [
    [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
];

/// Bradford chromatic adaptation from D50 to D65.
const D50_TO_D65: Mat3 = [
    [0.955473421488075, -0.0230984549487647, 0.0632592432005707],
    [-0.0283697093338637, 1.0099953980813, 0.0210414411919173],
    [0.012314014864482, -0.020507649298899, 1.33036592624212],
];

/// D50 reference white.
const D50_WHITE: Vec3 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

/// Linear Display P3 to XYZ (D65).
const LINEAR_P3_TO_XYZ: Mat3 = [
    [
        608311.0 / 1250200.0,
        189793.0 / 714400.0,
        198249.0 / 1000160.0,
    ],
    [
        35783.0 / 156275.0,
        247089.0 / 357200.0,
        198249.0 / 2500400.0,
    ],
    [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
];

/// Linear Rec. 2020 to XYZ (D65).
const LINEAR_REC2020_TO_XYZ: Mat3 = [
    [
        63426534.0 / 99577255.0,
        20160776.0 / 139408157.0,
        47086771.0 / 278816314.0,
    ],
    [
        26158966.0 / 99577255.0,
        472592308.0 / 697040785.0,
        8267143.0 / 139408157.0,
    ],
    [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
];

/// Linear A98 RGB to XYZ (D65).
const LINEAR_A98_TO_XYZ: Mat3 = [
    [
        573536.0 / 994567.0,
        263643.0 / 1420810.0,
        187206.0 / 994567.0,
    ],
    [
        591459.0 / 1989134.0,
        6239551.0 / 9945670.0,
        374412.0 / 4972835.0,
    ],
    [
        53769.0 / 1989134.0,
        351524.0 / 4972835.0,
        4929758.0 / 4972835.0,
    ],
];

/// Linear ProPhoto RGB to XYZ (D50).
const LINEAR_PROPHOTO_TO_XYZ_D50: Mat3 = [
    [0.797766644900642, 0.135181297400533, 0.0313477341283922],
    [0.288074828819401, 0.711835234241873, 0.0000899369387256],
    [0.0, 0.0, 0.82510460251046],
];

/// sRGB transfer function, decoding (gamma-encoded → linear).
pub(crate) fn srgb_to_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs <= 0.04045 {
        c / 12.92
    } else {
        c.signum() * ((abs + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function, encoding (linear → gamma-encoded).
pub(crate) fn linear_to_srgb(c: f64) -> f64 {
    let abs = c.abs();
    if abs > 0.0031308 {
        c.signum() * (1.055 * abs.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * c
    }
}

fn encode(linear: Vec3) -> Vec3 {
    linear.map(linear_to_srgb)
}

fn xyz_d65_to_srgb(xyz: Vec3) -> Vec3 {
    encode(mul(&XYZ_TO_LINEAR_SRGB, xyz))
}

fn xyz_d50_to_srgb(xyz: Vec3) -> Vec3 {
    xyz_d65_to_srgb(mul(&D50_TO_D65, xyz))
}

/// CIE Lab (D50) to sRGB.
pub(crate) fn lab_to_srgb(l: f64, a: f64, b: f64) -> Vec3 {
    const KAPPA: f64 = 24389.0 / 27.0;
    const EPSILON: f64 = 216.0 / 24389.0;
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let x = if f0.powi(3) > EPSILON {
        f0.powi(3)
    } else {
        (116.0 * f0 - 16.0) / KAPPA
    };
    let y = if l > KAPPA * EPSILON {
        f1.powi(3)
    } else {
        l / KAPPA
    };
    let z = if f2.powi(3) > EPSILON {
        f2.powi(3)
    } else {
        (116.0 * f2 - 16.0) / KAPPA
    };
    xyz_d50_to_srgb([x * D50_WHITE[0], y * D50_WHITE[1], z * D50_WHITE[2]])
}

/// LCH (D50) to sRGB; `h` in degrees.
pub(crate) fn lch_to_srgb(l: f64, c: f64, h: f64) -> Vec3 {
    let (sin, cos) = h.to_radians().sin_cos();
    lab_to_srgb(l, c * cos, c * sin)
}

/// OKLab to sRGB.
pub(crate) fn oklab_to_srgb(l: f64, a: f64, b: f64) -> Vec3 {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    encode([
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ])
}

/// OKLCH to sRGB; `h` in degrees.
pub(crate) fn oklch_to_srgb(l: f64, c: f64, h: f64) -> Vec3 {
    let (sin, cos) = h.to_radians().sin_cos();
    oklab_to_srgb(l, c * cos, c * sin)
}

/// Converts `color(<space> c1 c2 c3)` components to sRGB; `None` for an
/// unknown color space.
pub(crate) fn predefined_to_srgb(space: &str, c: Vec3) -> Option<Vec3> {
    Some(match space {
        "srgb" => c,
        "srgb-linear" => encode(c),
        "display-p3" => xyz_d65_to_srgb(mul(&LINEAR_P3_TO_XYZ, c.map(srgb_to_linear))),
        "a98-rgb" => {
            let linear = c.map(|v| v.signum() * v.abs().powf(563.0 / 256.0));
            xyz_d65_to_srgb(mul(&LINEAR_A98_TO_XYZ, linear))
        }
        "prophoto-rgb" => {
            let linear = c.map(|v| {
                if v.abs() <= 16.0 / 512.0 {
                    v / 16.0
                } else {
                    v.signum() * v.abs().powf(1.8)
                }
            });
            xyz_d50_to_srgb(mul(&LINEAR_PROPHOTO_TO_XYZ_D50, linear))
        }
        "rec2020" => {
            const ALPHA: f64 = 1.09929682680944;
            const BETA: f64 = 0.018053968510807;
            let linear = c.map(|v| {
                if v.abs() < BETA * 4.5 {
                    v / 4.5
                } else {
                    v.signum() * ((v.abs() + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
                }
            });
            xyz_d65_to_srgb(mul(&LINEAR_REC2020_TO_XYZ, linear))
        }
        "xyz" | "xyz-d65" => xyz_d65_to_srgb(c),
        "xyz-d50" => xyz_d50_to_srgb(c),
        _ => return None,
    })
}
