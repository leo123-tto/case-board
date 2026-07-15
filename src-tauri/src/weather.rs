//! 首页天气后端。
//!
//! HTTP 请求统一放在 Rust 侧，避免 Windows WebView 的 CSP 与跨域差异。
//! 前端能拿到系统定位时优先使用；拿不到时才退回 IP 粗略定位。

use serde::{Deserialize, Serialize};
use std::time::Duration;

const IP_LOCATION_URL: &str = "https://ipwho.is/";
const WEATHER_URL: &str = "https://api.open-meteo.com/v1/forecast";
const REQUEST_TIMEOUT_SECS: u64 = 8;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherRequest {
    latitude: Option<f64>,
    longitude: Option<f64>,
    warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeatherInfo {
    source: &'static str,
    label: Option<String>,
    summary: String,
    detail: String,
}

#[derive(Debug, Deserialize)]
struct IpLocation {
    success: Option<bool>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    city: Option<String>,
    region: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenMeteoResponse {
    current: Option<CurrentWeather>,
    daily: Option<DailyWeather>,
}

#[derive(Debug, Default, Deserialize)]
struct CurrentWeather {
    temperature_2m: Option<f64>,
    precipitation: Option<f64>,
    rain: Option<f64>,
    showers: Option<f64>,
    weather_code: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
struct DailyWeather {
    temperature_2m_max: Option<Vec<f64>>,
    temperature_2m_min: Option<Vec<f64>>,
    precipitation_probability_max: Option<Vec<f64>>,
    precipitation_sum: Option<Vec<f64>>,
    rain_sum: Option<Vec<f64>>,
}

struct ResolvedLocation {
    latitude: f64,
    longitude: f64,
    source: &'static str,
    label: Option<String>,
    warning: Option<String>,
}

#[tauri::command]
pub async fn get_weather_info(request: Option<WeatherRequest>) -> Result<WeatherInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(format!("CaseBoard/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("天气客户端创建失败: {error}"))?;

    let location = resolve_location(&client, request.unwrap_or_default()).await?;
    let response = client
        .get(WEATHER_URL)
        .query(&[
            ("latitude", format!("{:.4}", location.latitude)),
            ("longitude", format!("{:.4}", location.longitude)),
            (
                "current",
                "temperature_2m,precipitation,rain,showers,weather_code".to_string(),
            ),
            (
                "daily",
                "temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum,rain_sum".to_string(),
            ),
            ("timezone", "auto".to_string()),
            ("forecast_days", "1".to_string()),
        ])
        .send()
        .await
        .map_err(|error| format!("天气请求失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("天气服务返回错误: {error}"))?
        .json::<OpenMeteoResponse>()
        .await
        .map_err(|error| format!("天气数据解析失败: {error}"))?;

    Ok(build_weather_info(response, location))
}

async fn resolve_location(
    client: &reqwest::Client,
    request: WeatherRequest,
) -> Result<ResolvedLocation, String> {
    match (request.latitude, request.longitude) {
        (Some(latitude), Some(longitude)) => {
            validate_coordinates(latitude, longitude)?;
            Ok(ResolvedLocation {
                latitude,
                longitude,
                source: "系统定位",
                label: None,
                warning: None,
            })
        }
        (Some(_), None) | (None, Some(_)) => Err("系统定位返回的经纬度不完整".to_string()),
        (None, None) => {
            let response = client
                .get(IP_LOCATION_URL)
                .send()
                .await
                .map_err(|error| format!("网络定位请求失败: {error}"))?
                .error_for_status()
                .map_err(|error| format!("网络定位服务返回错误: {error}"))?
                .json::<IpLocation>()
                .await
                .map_err(|error| format!("网络定位数据解析失败: {error}"))?;
            if response.success == Some(false) {
                return Err(response
                    .message
                    .unwrap_or_else(|| "网络定位失败".to_string()));
            }
            let latitude = response
                .latitude
                .ok_or_else(|| "网络定位返回无纬度".to_string())?;
            let longitude = response
                .longitude
                .ok_or_else(|| "网络定位返回无经度".to_string())?;
            validate_coordinates(latitude, longitude)?;
            let label = [response.city, response.region]
                .into_iter()
                .flatten()
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Ok(ResolvedLocation {
                latitude,
                longitude,
                source: "网络定位",
                label: (!label.is_empty()).then_some(label),
                warning: request.warning.filter(|value| !value.trim().is_empty()),
            })
        }
    }
}

fn validate_coordinates(latitude: f64, longitude: f64) -> Result<(), String> {
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        return Err("定位纬度无效".to_string());
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        return Err("定位经度无效".to_string());
    }
    Ok(())
}

fn build_weather_info(data: OpenMeteoResponse, location: ResolvedLocation) -> WeatherInfo {
    let daily = data.daily.unwrap_or_default();
    let current = data.current.unwrap_or_default();
    let current_precipitation = current
        .precipitation
        .or(current.rain)
        .or(current.showers)
        .unwrap_or(0.0);
    let probability = first(daily.precipitation_probability_max).unwrap_or(0.0);
    let precipitation = first(daily.precipitation_sum)
        .or_else(|| first(daily.rain_sum))
        .unwrap_or(0.0);
    let has_current_rain =
        current_precipitation > 0.1 || is_rain_weather_code(current.weather_code);
    let has_rain = has_current_rain || precipitation > 0.1 || probability >= 30.0;
    let current_text = current
        .temperature_2m
        .map(|temperature| format!("现在 {:.0}°C", temperature));
    let day_text = match (
        first(daily.temperature_2m_min),
        first(daily.temperature_2m_max),
    ) {
        (Some(min), Some(max)) => Some(format!("今日 {:.0}-{:.0}°C", min, max)),
        _ => None,
    };
    let rain_text = if has_current_rain {
        "正在下雨"
    } else if has_rain {
        "可能有雨"
    } else {
        "少雨"
    };
    let place = location.label.as_deref().unwrap_or(location.source);
    let summary = [
        Some(place.to_string()),
        current_text,
        day_text,
        Some(rain_text.to_string()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let label_detail = location
        .label
        .as_ref()
        .map(|label| format!(" · {label}"))
        .unwrap_or_default();
    let warning_detail = location
        .warning
        .as_ref()
        .map(|warning| format!(" · 系统定位失败: {warning}"))
        .unwrap_or_default();
    let detail = format!(
        "{}{}{} · 当前降水 {:.1}mm · 降雨概率 {:.0}% · 预计降雨 {:.1}mm",
        location.source,
        label_detail,
        warning_detail,
        current_precipitation,
        probability,
        precipitation
    );

    WeatherInfo {
        source: location.source,
        label: location.label,
        summary,
        detail,
    }
}

fn first(values: Option<Vec<f64>>) -> Option<f64> {
    values.and_then(|values| values.into_iter().next())
}

fn is_rain_weather_code(code: Option<u16>) -> bool {
    matches!(code, Some(51..=67 | 80..=82 | 95..=99))
}
