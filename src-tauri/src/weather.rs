/**
 * 首页看板助手 — 天气 + 位置获取(Rust 后端)
 *
 * 背景:前端 fetch 受 Tauri WebView2 的 CSP / user agent / 网络栈影响,即使
 * 修 CSP 后位置/天气仍可能失败。改用 Rust reqwest 直接调,跨平台行为一致,
 * 错误信息详细可控。
 *
 * 数据源:
 * - IP 定位:https://ipwho.is/ (免 key,JSON,latitude/longitude + city/region)
 *   注:ipapi.co 一直 403 Forbidden,弃用
 * - 天气:https://api.open-meteo.com/v1/forecast (免 key,WMO weather code)
 *
 * 调用方式:invoke("get_weather_info") 一个 Tauri command,内部串行两步。
 */
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const IP_URL: &str = "https://ipwho.is/";
const WEATHER_URL: &str = "https://api.open-meteo.com/v1/forecast";
const USER_AGENT: &str = concat!("CaseBoard/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize)]
pub struct WeatherInfo {
    pub source: String,
    pub label: Option<String>,
    pub summary: String,
    pub detail: String,
    pub warning: Option<String>,
    pub current_temp: Option<f64>,
    pub temp_min: Option<f64>,
    pub temp_max: Option<f64>,
    pub precipitation_probability: f64,
    pub precipitation_sum: f64,
    pub current_precipitation: f64,
    pub weather_code: Option<i32>,
    pub latitude: f64,
    pub longitude: f64,
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))
}

#[derive(Debug, Deserialize)]
struct IpWhoResponse {
    success: Option<bool>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    city: Option<String>,
    region: Option<String>,
    message: Option<String>,
}

async fn resolve_ip_location(
    client: &reqwest::Client,
) -> Result<(f64, f64, Option<String>, Option<String>), String> {
    let resp = client
        .get(IP_URL)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("IP 定位请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("IP 定位返回 HTTP {}", resp.status().as_u16()));
    }
    let data: IpWhoResponse = resp
        .json()
        .await
        .map_err(|e| format!("IP 定位响应解析失败: {}", e))?;
    if data.success == Some(false) {
        return Err(data
            .message
            .unwrap_or_else(|| "IP 定位返回失败".to_string()));
    }
    let lat = data
        .latitude
        .ok_or_else(|| "IP 定位无经度".to_string())?;
    let lon = data
        .longitude
        .ok_or_else(|| "IP 定位无纬度".to_string())?;
    Ok((lat, lon, data.city, data.region))
}

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrent {
    #[serde(default)]
    temperature_2m: Option<f64>,
    #[serde(default)]
    precipitation: Option<f64>,
    #[serde(default)]
    rain: Option<f64>,
    #[serde(default)]
    showers: Option<f64>,
    #[serde(default)]
    weather_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoDaily {
    #[serde(default)]
    temperature_2m_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    temperature_2m_min: Option<Vec<Option<f64>>>,
    #[serde(default)]
    precipitation_probability_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    precipitation_sum: Option<Vec<Option<f64>>>,
    #[serde(default)]
    rain_sum: Option<Vec<Option<f64>>>,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    #[serde(default)]
    current: Option<OpenMeteoCurrent>,
    #[serde(default)]
    daily: Option<OpenMeteoDaily>,
}

/// 单一 Tauri command:IP 定位 + 天气 fetch + 中文 summary。
/// 前端 invoke('get_weather_info') 一个调用,不依赖 webview 的 CSP / fetch。
#[tauri::command]
pub async fn get_weather_info() -> Result<WeatherInfo, String> {
    let client = build_client()?;
    let (lat, lon, city, region) = resolve_ip_location(&client)
        .await
        .map_err(|e| format!("位置获取失败: {}", e))?;

    let resp = client
        .get(WEATHER_URL)
        .query(&json!({
            "latitude": lat,
            "longitude": lon,
            "current": "temperature_2m,precipitation,rain,showers,weather_code",
            "daily": "temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum,rain_sum",
            "timezone": "auto",
            "forecast_days": 1,
        }))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("天气请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("天气返回 HTTP {}", resp.status().as_u16()));
    }
    let data: OpenMeteoResponse = resp
        .json()
        .await
        .map_err(|e| format!("天气响应解析失败: {}", e))?;

    let current = data.current.unwrap_or(OpenMeteoCurrent {
        temperature_2m: None,
        precipitation: None,
        rain: None,
        showers: None,
        weather_code: None,
    });
    let daily = data.daily.unwrap_or(OpenMeteoDaily {
        temperature_2m_max: None,
        temperature_2m_min: None,
        precipitation_probability_max: None,
        precipitation_sum: None,
        rain_sum: None,
    });

    let current_temp = current.temperature_2m;
    let current_precip = current
        .precipitation
        .or(current.rain)
        .or(current.showers)
        .unwrap_or(0.0);
    let temp_max = daily
        .temperature_2m_max
        .as_ref()
        .and_then(|v| v.first().copied().flatten());
    let temp_min = daily
        .temperature_2m_min
        .as_ref()
        .and_then(|v| v.first().copied().flatten());
    let prob = daily
        .precipitation_probability_max
        .as_ref()
        .and_then(|v| v.first().copied().flatten())
        .unwrap_or(0.0);
    let precip_sum = daily
        .precipitation_sum
        .as_ref()
        .and_then(|v| v.first().copied().flatten())
        .or_else(|| {
            daily
                .rain_sum
                .as_ref()
                .and_then(|v| v.first().copied().flatten())
        })
        .unwrap_or(0.0);
    let weather_code = current.weather_code;

    let rain_desc = if current_precip > 0.1 {
        "正在下雨"
    } else if prob >= 30.0 || precip_sum > 0.1 {
        "可能有雨"
    } else {
        "少雨"
    };

    let place = city
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("网络定位")
        .to_string();
    let mut summary_parts: Vec<String> = vec![place.clone()];
    if let Some(t) = current_temp {
        summary_parts.push(format!("现在 {}°C", t.round() as i32));
    }
    if let (Some(mn), Some(mx)) = (temp_min, temp_max) {
        summary_parts.push(format!(
            "今日 {}~{}°C",
            mn.round() as i32,
            mx.round() as i32
        ));
    }
    summary_parts.push(rain_desc.to_string());
    let summary = summary_parts.join(" · ");

    let label = match (city.as_deref(), region.as_deref()) {
        (Some(c), Some(r)) if !c.is_empty() && !r.is_empty() => Some(format!("{} · {}", c, r)),
        (Some(c), _) if !c.is_empty() => Some(c.to_string()),
        _ => None,
    };

    Ok(WeatherInfo {
        source: "网络定位".to_string(),
        label,
        summary,
        detail: format!(
            "网络定位 · 当前位置降水 {:.1}mm · 降雨概率 {}% · 预计降雨 {:.1}mm",
            current_precip,
            prob.round() as i32,
            precip_sum
        ),
        warning: None,
        current_temp,
        temp_min,
        temp_max,
        precipitation_probability: prob,
        precipitation_sum: precip_sum,
        current_precipitation: current_precip,
        weather_code,
        latitude: lat,
        longitude: lon,
    })
}
