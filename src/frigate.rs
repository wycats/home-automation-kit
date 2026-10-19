use reqwest::{Url, blocking::Client, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, fmt, io::Read, time::Duration};

const MAX_RESPONSE_BYTES: u64 = 1_048_576;

/// Errors deliberately omit response bodies, URLs, tokens, and transport details.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput(&'static str),
    Transport,
    Http(u16),
    ResponseTooLarge,
    InvalidResponse,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(f, "{message}"),
            Self::Transport => write!(f, "Frigate request failed"),
            Self::Http(status) => write!(f, "Frigate returned HTTP {status}"),
            Self::ResponseTooLarge => write!(f, "Frigate response exceeds the size limit"),
            Self::InvalidResponse => write!(f, "Frigate returned an invalid response"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Deserialize)]
pub struct Stats {
    #[serde(default)]
    pub cameras: BTreeMap<String, CameraStats>,
}

#[derive(Debug, Deserialize)]
pub struct CameraStats {
    pub camera_fps: Option<f64>,
    pub process_fps: Option<f64>,
    pub skipped_fps: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Recording {
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordingFreshness {
    Recent {
        age_seconds: f64,
    },
    Stale {
        age_seconds: f64,
    },
    /// Absence in this bounded query is not proof that no recordings exist.
    NoSegmentsInWindow,
    InvalidTimestamps,
}

#[derive(Debug, Serialize)]
pub struct Observation {
    pub observed_at_unix_seconds: f64,
    pub window_start_unix_seconds: f64,
    pub camera: String,
    pub present_in_stats: bool,
    pub camera_fps: Option<f64>,
    pub process_fps: Option<f64>,
    pub skipped_fps: Option<f64>,
    pub recording_freshness: RecordingFreshness,
    pub segments_in_window: usize,
}

/// Validates the whole sample rather than silently dropping malformed segments.
/// Future timestamps are inconclusive: there may be clock skew.
pub fn recording_freshness(
    recordings: &[Recording],
    now: f64,
    max_age_seconds: f64,
) -> Result<RecordingFreshness, Error> {
    if !now.is_finite() || now < 0.0 || !max_age_seconds.is_finite() || max_age_seconds < 0.0 {
        return Err(Error::InvalidInput(
            "invalid observation time or freshness threshold",
        ));
    }
    if recordings.is_empty() {
        return Ok(RecordingFreshness::NoSegmentsInWindow);
    }
    if recordings.iter().any(|r| {
        !r.start_time.is_finite()
            || !r.end_time.is_finite()
            || r.start_time < 0.0
            || r.end_time < r.start_time
            || r.end_time > now
    }) {
        return Ok(RecordingFreshness::InvalidTimestamps);
    }
    let latest = recordings.iter().map(|r| r.end_time).fold(0.0, f64::max);
    let age_seconds = now - latest;
    Ok(if age_seconds <= max_age_seconds {
        RecordingFreshness::Recent { age_seconds }
    } else {
        RecordingFreshness::Stale { age_seconds }
    })
}

/// A bounded HTTP client. Redirects are rejected, including same-host redirects.
/// TLS verification remains enabled; only GET endpoints are exposed.
pub struct FrigateClient {
    base: Url,
    client: Client,
}

impl FrigateClient {
    pub fn new(base: &str, bearer_token: Option<&str>, timeout: Duration) -> Result<Self, Error> {
        let mut base =
            Url::parse(base).map_err(|_| Error::InvalidInput("invalid Frigate base URL"))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || timeout.is_zero()
        {
            return Err(Error::InvalidInput("invalid Frigate base URL or timeout"));
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let mut headers = header::HeaderMap::new();
        if let Some(token) = bearer_token {
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Error::InvalidInput("invalid bearer token"))?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .build()
            .map_err(|_| Error::Transport)?;
        Ok(Self { base, client })
    }

    fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T, Error> {
        let response = self.client.get(url).send().map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES)
        {
            return Err(Error::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Transport)?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
    }

    pub fn stats(&self) -> Result<Stats, Error> {
        self.get(
            self.base
                .join("api/stats")
                .map_err(|_| Error::InvalidInput("invalid base path"))?,
        )
    }

    pub fn recordings(
        &self,
        camera: &str,
        after: f64,
        before: f64,
    ) -> Result<Vec<Recording>, Error> {
        if camera.is_empty()
            || !camera
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(Error::InvalidInput(
                "camera must contain only letters, digits, or underscores",
            ));
        }
        if !after.is_finite() || !before.is_finite() || after < 0.0 || before <= after {
            return Err(Error::InvalidInput("invalid recording time window"));
        }
        let mut url = self
            .base
            .join(&format!("api/{camera}/recordings"))
            .map_err(|_| Error::InvalidInput("invalid base path"))?;
        url.query_pairs_mut()
            .append_pair("after", &after.to_string())
            .append_pair("before", &before.to_string());
        self.get(url)
    }

    /// Fetches a bounded window, without asserting that metadata proves playable video.
    pub fn observe(
        &self,
        camera: &str,
        now: f64,
        window_seconds: f64,
        max_age: f64,
    ) -> Result<Observation, Error> {
        if !window_seconds.is_finite()
            || window_seconds <= 0.0
            || !now.is_finite()
            || now < window_seconds
        {
            return Err(Error::InvalidInput("invalid observation window"));
        }
        // Validate the threshold before making network requests.
        recording_freshness(&[], now, max_age)?;
        let records = self.recordings(camera, now - window_seconds, now)?;
        let stats = self.stats()?;
        let camera_stats = stats.cameras.get(camera);
        let valid_rate = |value: Option<f64>| value.filter(|v| v.is_finite() && *v >= 0.0);
        Ok(Observation {
            observed_at_unix_seconds: now,
            window_start_unix_seconds: now - window_seconds,
            camera: camera.to_string(),
            present_in_stats: camera_stats.is_some(),
            camera_fps: valid_rate(camera_stats.and_then(|c| c.camera_fps)),
            process_fps: valid_rate(camera_stats.and_then(|c| c.process_fps)),
            skipped_fps: valid_rate(camera_stats.and_then(|c| c.skipped_fps)),
            recording_freshness: recording_freshness(&records, now, max_age)?,
            segments_in_window: records.len(),
        })
    }
}
