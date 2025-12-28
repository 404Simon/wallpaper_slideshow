use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct ExifInfo {
    pub datetime: Option<String>,
    pub datetime_raw: Option<String>,
    pub hour: Option<u8>,
    pub location: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub exposure: Option<String>,
    pub aperture: Option<String>,
    pub iso: Option<String>,
    pub focal_length: Option<String>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
}

impl ExifInfo {
    pub fn has_gps(&self) -> bool {
        self.gps_latitude.is_some() && self.gps_longitude.is_some()
    }

    pub fn maps_url(&self) -> Option<String> {
        match (self.gps_latitude, self.gps_longitude) {
            (Some(lat), Some(lon)) => {
                Some(format!("https://maps.google.com/?q={:.6},{:.6}", lat, lon))
            }
            _ => None,
        }
    }
}

pub fn extract(path: &Path) -> ExifInfo {
    let mut info = ExifInfo::default();

    let exif = match rexif::parse_file(path) {
        Ok(e) => e,
        Err(_) => return info,
    };

    let mut gps = GpsData::default();

    for entry in &exif.entries {
        match entry.tag {
            rexif::ExifTag::DateTimeOriginal => {
                if let rexif::TagValue::Ascii(ref s) = entry.value {
                    info.datetime_raw = Some(s.clone());
                    info.datetime = Some(format_datetime(s));
                    info.hour = parse_hour_from_datetime(s);
                }
            }
            rexif::ExifTag::Make => {
                if let rexif::TagValue::Ascii(ref s) = entry.value {
                    info.camera = Some(s.trim().to_string());
                }
            }
            rexif::ExifTag::Model => {
                if let rexif::TagValue::Ascii(ref s) = entry.value {
                    let model = s.trim().to_string();
                    info.camera = Some(match &info.camera {
                        Some(make) if !model.starts_with(make) => format!("{} {}", make, model),
                        _ => model,
                    });
                }
            }
            rexif::ExifTag::LensModel => {
                if let rexif::TagValue::Ascii(ref s) = entry.value {
                    info.lens = Some(s.trim().to_string());
                }
            }
            rexif::ExifTag::ExposureTime => {
                info.exposure = Some(entry.value_more_readable.to_string());
            }
            rexif::ExifTag::FNumber => {
                if let rexif::TagValue::URational(ref vals) = entry.value {
                    if let Some(v) = vals.first() {
                        let f = v.numerator as f64 / v.denominator as f64;
                        info.aperture = Some(format!("f/{:.1}", f));
                    }
                }
            }
            rexif::ExifTag::ISOSpeedRatings => {
                if let rexif::TagValue::U16(ref vals) = entry.value {
                    if let Some(&iso) = vals.first() {
                        info.iso = Some(format!("ISO {}", iso));
                    }
                }
            }
            rexif::ExifTag::FocalLength => {
                if let rexif::TagValue::URational(ref vals) = entry.value {
                    if let Some(v) = vals.first() {
                        let fl = v.numerator as f64 / v.denominator as f64;
                        info.focal_length = Some(format!("{:.0}mm", fl));
                    }
                }
            }
            rexif::ExifTag::GPSLatitude => gps.parse_lat(&entry.value),
            rexif::ExifTag::GPSLatitudeRef => gps.parse_lat_ref(&entry.value),
            rexif::ExifTag::GPSLongitude => gps.parse_lon(&entry.value),
            rexif::ExifTag::GPSLongitudeRef => gps.parse_lon_ref(&entry.value),
            _ => {}
        }
    }

    if let Some((lat, lon)) = gps.to_decimal() {
        info.gps_latitude = Some(lat);
        info.gps_longitude = Some(lon);
        info.location = Some(format_gps_coordinates(lat, lon));
    }

    info
}

#[derive(Default)]
struct GpsData {
    lat: Option<(f64, f64, f64)>,
    lat_ref: Option<String>,
    lon: Option<(f64, f64, f64)>,
    lon_ref: Option<String>,
}

impl GpsData {
    fn parse_lat(&mut self, value: &rexif::TagValue) {
        if let rexif::TagValue::URational(vals) = value {
            if vals.len() >= 3 {
                self.lat = Some((
                    vals[0].numerator as f64 / vals[0].denominator as f64,
                    vals[1].numerator as f64 / vals[1].denominator as f64,
                    vals[2].numerator as f64 / vals[2].denominator as f64,
                ));
            }
        }
    }

    fn parse_lat_ref(&mut self, value: &rexif::TagValue) {
        if let rexif::TagValue::Ascii(ref s) = value {
            self.lat_ref = Some(s.clone());
        }
    }

    fn parse_lon(&mut self, value: &rexif::TagValue) {
        if let rexif::TagValue::URational(vals) = value {
            if vals.len() >= 3 {
                self.lon = Some((
                    vals[0].numerator as f64 / vals[0].denominator as f64,
                    vals[1].numerator as f64 / vals[1].denominator as f64,
                    vals[2].numerator as f64 / vals[2].denominator as f64,
                ));
            }
        }
    }

    fn parse_lon_ref(&mut self, value: &rexif::TagValue) {
        if let rexif::TagValue::Ascii(ref s) = value {
            self.lon_ref = Some(s.clone());
        }
    }

    fn to_decimal(&self) -> Option<(f64, f64)> {
        let (lat_d, lat_m, lat_s) = self.lat?;
        let lat_ref = self.lat_ref.as_ref()?;
        let (lon_d, lon_m, lon_s) = self.lon?;
        let lon_ref = self.lon_ref.as_ref()?;

        let mut lat = lat_d + lat_m / 60.0 + lat_s / 3600.0;
        let mut lon = lon_d + lon_m / 60.0 + lon_s / 3600.0;

        if lat_ref == "S" {
            lat = -lat;
        }
        if lon_ref == "W" {
            lon = -lon;
        }

        Some((lat, lon))
    }
}

/// format: "YYYY:MM:DD HH:MM:SS"
fn parse_hour_from_datetime(datetime: &str) -> Option<u8> {
    if datetime.len() >= 13 {
        let hour_str = &datetime[11..13];
        if let Ok(hour) = hour_str.parse::<u8>() {
            if hour <= 23 {
                return Some(hour);
            }
        }
    }
    None
}

fn format_datetime(s: &str) -> String {
    if s.len() < 19 {
        return s.to_string();
    }

    let month_name = match &s[5..7] {
        "01" => "January",
        "02" => "February",
        "03" => "March",
        "04" => "April",
        "05" => "May",
        "06" => "June",
        "07" => "July",
        "08" => "August",
        "09" => "September",
        "10" => "October",
        "11" => "November",
        "12" => "December",
        m => m,
    };

    let day: u32 = s[8..10].parse().unwrap_or(0);
    format!(
        "{} {}, {} at {}:{}",
        month_name,
        day,
        &s[0..4],
        &s[11..13],
        &s[14..16]
    )
}

fn format_gps_coordinates(lat: f64, lon: f64) -> String {
    let (lat_dir, lon_dir) = (
        if lat >= 0.0 { "N" } else { "S" },
        if lon >= 0.0 { "E" } else { "W" },
    );
    let (lat_abs, lon_abs) = (lat.abs(), lon.abs());

    let format_dms = |v: f64| {
        let deg = v.floor();
        let min = ((v - deg) * 60.0).floor();
        let sec = ((v - deg) * 60.0 - min) * 60.0;
        (deg, min, sec)
    };

    let (lat_d, lat_m, lat_s) = format_dms(lat_abs);
    let (lon_d, lon_m, lon_s) = format_dms(lon_abs);

    format!(
        "{:.0}deg{:.0}'{:.1}\"{} {:.0}deg{:.0}'{:.1}\"{}",
        lat_d, lat_m, lat_s, lat_dir, lon_d, lon_m, lon_s, lon_dir
    )
}
