//! GIS service-type-specific probe checks (task 2.3).
//!
//! The transport layer ([`super::client`]) answers "did HTTP work?"; this
//! module answers "does the GIS service behave?":
//!
//! | ServiceType | Strategy |
//! |-------------|----------|
//! | WMS / WFS / WMTS | `GetCapabilities` request; expect XML capabilities doc, no `ServiceException` |
//! | OAF | landing page JSON with a `links` array (OGC API — Features) |
//! | ArcGIS_REST | `?f=pjson` health ping; JSON without an `error` key |
//! | HTTP | no extra checks — transport success is enough |

use crate::db::models::ServiceType;
use crate::queue::ProbeJob;

/// Check the (successful-transport) response body at the service level.
///
/// `body` is the full response text. Returns `Err(reason)` when the service
/// reports an error despite a healthy transport (e.g. HTTP 200 carrying a
/// WMS `ServiceException`).
pub fn check_service_response(service_type: ServiceType, body: &str) -> Result<(), String> {
    match service_type {
        ServiceType::Wms | ServiceType::Wfs | ServiceType::Wmts => check_ogc_capabilities(body),
        ServiceType::Oaf => check_oaf_landing(body),
        ServiceType::ArcGisRest => check_arcgis_json(body),
        ServiceType::Http => Ok(()),
    }
}

/// OGC capabilities check: no `ServiceException`, root element is a
/// capabilities document (`*_Capabilities` / `WMS_Capabilities` / ...).
fn check_ogc_capabilities(body: &str) -> Result<(), String> {
    if body.contains("ServiceExceptionReport") || body.contains("<ServiceException") {
        return Err("service returned a ServiceException".into());
    }

    match root_element_name(body) {
        Some(root) => {
            if root.to_ascii_lowercase().contains("capabilities") {
                Ok(())
            } else if root.eq_ignore_ascii_case("ExceptionReport") {
                Err("service returned an ExceptionReport".into())
            } else {
                Err(format!(
                    "unexpected root element <{root}>, expected a capabilities document"
                ))
            }
        }
        None => Err("response is not parseable XML (no root element found)".into()),
    }
}

/// OGC API — Features landing page: JSON object with a `links` array.
fn check_oaf_landing(body: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "response is not valid JSON (expected OAF landing page)".to_string())?;
    match value.get("links") {
        Some(links) if links.is_array() => Ok(()),
        Some(_) => Err("OAF landing page 'links' is not an array".into()),
        None => Err("OAF landing page has no 'links' array".into()),
    }
}

/// ArcGIS Server health ping: JSON without an `error` key.
fn check_arcgis_json(body: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "response is not valid JSON (expected ArcGIS f=pjson reply)".to_string())?;
    if value.get("error").is_some() {
        return Err("ArcGIS reply contains an 'error' object".into());
    }
    Ok(())
}

/// Extract the name of the root element of an XML document, tolerating an
/// XML declaration, comments, DOCTYPE, and leading whitespace.
fn root_element_name(xml: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => {
                return Some(String::from_utf8_lossy(e.name().as_ref()).into_owned());
            }
            Ok(quick_xml::events::Event::Eof) => return None,
            Ok(_) => continue, // declaration, comment, text, ...
            Err(_) => return None,
        }
    }
}

/// Build the concrete request URL for a probe job.
///
/// GIS-specific query parameters are appended/overridden:
/// `service`+`request=GetCapabilities` for WMS/WFS/WMTS, `f=pjson` for
/// ArcGIS_REST; OAF and HTTP use the configured URL as-is.
pub fn build_probe_url(job: &ProbeJob) -> String {
    let mut url = match url::Url::parse(&job.url) {
        Ok(u) => u,
        // Validation should have caught this; fall back to the raw URL and
        // let the transport layer surface the error.
        Err(_) => return job.url.clone(),
    };

    // Drop conflicting configured params (case-insensitively — OGC params
    // arrive as `SERVICE`/`REQUEST` just as often as lowercase), then append
    // the canonical ones.
    let managed: Vec<String> = match job.service_type {
        ServiceType::Wms | ServiceType::Wfs | ServiceType::Wmts => {
            vec!["service".into(), "request".into()]
        }
        ServiceType::ArcGisRest => vec!["f".into()],
        ServiceType::Oaf | ServiceType::Http => return url.to_string(),
    };
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !managed.contains(&k.to_ascii_lowercase()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut().clear().extend_pairs(pairs);

    {
        let mut qp = url.query_pairs_mut();
        match job.service_type {
            ServiceType::Wms => {
                qp.append_pair("service", "WMS");
                qp.append_pair("request", "GetCapabilities");
            }
            ServiceType::Wfs => {
                qp.append_pair("service", "WFS");
                qp.append_pair("request", "GetCapabilities");
            }
            ServiceType::Wmts => {
                qp.append_pair("service", "WMTS");
                qp.append_pair("request", "GetCapabilities");
            }
            ServiceType::ArcGisRest => {
                qp.append_pair("f", "json");
            }
            ServiceType::Oaf | ServiceType::Http => {}
        }
    }

    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn job(service: ServiceType, url: &str) -> ProbeJob {
        ProbeJob {
            target_id: uuid::Uuid::nil(),
            target_name: "t".into(),
            url: url.into(),
            service_type: service,
            expected_time_ms: 500,
            timeout_ms: 5000,
            http_method: crate::db::models::HttpMethod::Get,
            custom_headers: HashMap::new(),
            auth: None,
        }
    }

    #[test]
    fn wms_url_gets_capabilities_params() {
        let u = build_probe_url(&job(
            ServiceType::Wms,
            "https://host/geoserver/wms?SERVICE=WMS",
        ));
        assert_eq!(
            u,
            "https://host/geoserver/wms?service=WMS&request=GetCapabilities"
        );
    }

    #[test]
    fn arcgis_url_gets_pjson() {
        let u = build_probe_url(&job(
            ServiceType::ArcGisRest,
            "https://host/arcgis/rest/services",
        ));
        assert_eq!(u, "https://host/arcgis/rest/services?f=json");
    }

    #[test]
    fn plain_http_url_untouched() {
        let u = build_probe_url(&job(ServiceType::Http, "https://host/health?x=1"));
        assert_eq!(u, "https://host/health?x=1");
    }

    #[test]
    fn wms_capabilities_ok_and_exceptions_fail() {
        let good = r#"<?xml version="1.0"?><WMS_Capabilities version="1.3.0"></WMS_Capabilities>"#;
        assert!(check_service_response(ServiceType::Wms, good).is_ok());
        let svc_exc = r#"<?xml version="1.0"?><ServiceExceptionReport><ServiceException>boom</ServiceException></ServiceExceptionReport>"#;
        assert!(check_service_response(ServiceType::Wms, svc_exc).is_err());
        let wrong_root = r#"<?xml version="1.0"?><html><body>hi</body></html>"#;
        assert!(check_service_response(ServiceType::Wfs, wrong_root).is_err());
    }

    #[test]
    fn oaf_requires_links_array() {
        assert!(check_service_response(
            ServiceType::Oaf,
            r#"{"title":"x","links":[{"href":"h"}]}"#
        )
        .is_ok());
        assert!(check_service_response(ServiceType::Oaf, r#"{"title":"x"}"#).is_err());
        assert!(check_service_response(ServiceType::Oaf, "not json").is_err());
    }

    #[test]
    fn arcgis_error_key_fails() {
        assert!(
            check_service_response(ServiceType::ArcGisRest, r#"{"currentVersion":11}"#).is_ok()
        );
        assert!(
            check_service_response(ServiceType::ArcGisRest, r#"{"error":{"code":404}}"#).is_err()
        );
    }
}
