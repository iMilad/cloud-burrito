//! CodeArtifact Packages - list PyPI packages matching a prefix and show the
//! latest published version for each package.

use aws_sdk_codeartifact::{
    types::{PackageFormat, PackageVersionSortType, PackageVersionStatus, PackageVersionSummary},
    Client,
};
use aws_smithy_types::{date_time::Format, DateTime};
use futures::{stream, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;

use super::coverage::Coverage;
use super::{dt_iso, err_msg, WidgetCtx};

const DEFAULT_DOMAIN: &str = "example-domain";
const DEFAULT_REPOSITORY: &str = "example_pypi_repo";
const DEFAULT_PACKAGE_PREFIX: &str = "example";
const DEFAULT_MAX_PACKAGES: i64 = 50;
const MAX_PACKAGES: usize = 1000;
const RECENT_VERSION_LIMIT: usize = 10;
const VERSION_DETAIL_CONCURRENCY: usize = 4;

#[derive(Debug)]
struct PackageRow {
    package: String,
    latest_version: String,
    last_published: String,
    versions: Vec<String>,
    detail_failed: bool,
    detail_denied: bool,
    versions_limited: bool,
}

impl PackageRow {
    fn without_details(package: String) -> Self {
        Self {
            package,
            latest_version: String::new(),
            last_published: String::new(),
            versions: Vec::new(),
            detail_failed: false,
            detail_denied: false,
            versions_limited: false,
        }
    }
}

struct PackageListing {
    packages: Vec<String>,
    coverage: Coverage,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionSeed {
    version: String,
    published: String,
}

pub async fn fetch(ctx: &WidgetCtx) -> Value {
    let domain = ctx.input_str("domain", DEFAULT_DOMAIN).trim().to_string();
    let repository = ctx
        .input_str("repository", DEFAULT_REPOSITORY)
        .trim()
        .to_string();
    let package_prefix = ctx
        .input_str("package_prefix", DEFAULT_PACKAGE_PREFIX)
        .trim()
        .to_string();
    let max_packages = ctx
        .input_i64("max_packages", DEFAULT_MAX_PACKAGES)
        .clamp(1, MAX_PACKAGES as i64) as usize;
    let domain_owner = ctx.input_str("domain_owner", "").trim().to_string();

    if domain.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "domain input is required"}});
    }
    if repository.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "repository input is required"}});
    }
    if package_prefix.is_empty() {
        return json!({"render": "raw_json", "data": {"error": "package_prefix input is required"}});
    }

    let client = Client::new(&ctx.sdk);

    let listing = match list_packages(
        ctx,
        &client,
        &domain,
        &repository,
        &package_prefix,
        &domain_owner,
        max_packages,
    )
    .await
    {
        Ok(p) => p,
        Err(render) => return render,
    };

    let mut out: Vec<PackageRow> = Vec::with_capacity(listing.packages.len());
    let mut enrichment_denied = false;
    let mut attempted = 0usize;
    for package in &listing.packages {
        if listing.error.is_some() || enrichment_denied {
            out.push(PackageRow::without_details(package.clone()));
            continue;
        }
        match latest_package_row(
            ctx,
            &client,
            &domain,
            &repository,
            package.clone(),
            &domain_owner,
        )
        .await
        {
            Ok(row) => {
                attempted += 1;
                enrichment_denied = row.detail_denied;
                out.push(row);
            }
            Err(_) => {
                enrichment_denied = true;
                out.push(PackageRow::without_details(package.clone()));
            }
        }
    }
    let failed = out.iter().filter(|row| row.detail_failed).count();
    let limited_versions = out.iter().filter(|row| row.versions_limited).count();
    let mut enrichment = Coverage::complete(attempted - failed);
    enrichment.count("attempted", attempted);
    enrichment.count("failed", failed);
    enrichment.count("skipped", out.len() - attempted);
    enrichment.count("limited_version_lists", limited_versions);
    enrichment.limit("versions_per_package", Some(RECENT_VERSION_LIMIT));
    if failed > 0 {
        enrichment.failure(
            "enrichment_failed",
            "Some package version details could not be loaded.",
            true,
        );
    }
    if enrichment_denied {
        enrichment.failure(
            "policy_denied",
            "Additional package version details were not permitted.",
            !out.is_empty(),
        );
    } else if listing.error.is_some() {
        enrichment.unknown_reason(
            "not_attempted",
            "Version details were not requested after the package listing failed.",
        );
    }
    if limited_versions > 0 {
        enrichment.has_more(Some(true));
        enrichment.limited(
            "version_limit",
            "Only the recent version snapshot is shown for packages with additional versions.",
        );
    }
    let mut coverage = Coverage::complete(out.len());
    coverage.section("packages", listing.coverage);
    coverage.section("enrichment", enrichment);
    let mut result = package_table(out);
    if let Some(error) = listing.error {
        result["error"] = json!(error);
    }
    coverage.attach(result)
}

fn package_table(rows: Vec<PackageRow>) -> Value {
    let failed_count = rows.iter().filter(|row| row.detail_failed).count();
    let mut out: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "package": row.package,
                "latest_version": row.latest_version,
                "last_published": row.last_published,
                "versions": row.versions,
            })
        })
        .collect();
    out.sort_by_key(|row| {
        row.get("package")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase()
    });

    with_partial_failure(
        json!({
            "render": "table",
            "columns": ["package", "latest_version", "last_published"],
            "rows": out,
        }),
        failed_count,
    )
}

/// Resolve publish dates for one package's version snapshot. The package list
/// already supplies the ordered version strings; dates are enriched lazily so
/// the initial widget load does not multiply DescribePackageVersion calls by
/// every package in the table.
pub async fn fetch_version_history(ctx: &WidgetCtx) -> Value {
    let domain = ctx.input_str("domain", "").trim().to_string();
    let repository = ctx.input_str("repository", "").trim().to_string();
    let package = ctx.input_str("package", "").trim().to_string();
    let domain_owner = ctx.input_str("domain_owner", "").trim().to_string();

    if domain.is_empty() || repository.is_empty() || package.is_empty() {
        return version_history_error(
            &package,
            "domain, repository, and package inputs are required".to_string(),
        );
    }

    let seeds = version_seeds(ctx.input_value("versions"));
    if seeds.is_empty() {
        return version_history_error(&package, "no package versions were provided".to_string());
    }

    let client = Client::new(&ctx.sdk);
    let mut rows: Vec<(usize, Value, bool)> = Vec::with_capacity(seeds.len());
    let mut requests = Vec::new();

    for (index, seed) in seeds.into_iter().enumerate() {
        if !seed.published.is_empty() {
            rows.push((
                index,
                json!({"version": seed.version, "published": seed.published}),
                false,
            ));
            continue;
        }

        if let Some(denied) = ctx.preflight("codeartifact", "DescribePackageVersion") {
            return denied;
        }

        let mut request = client
            .describe_package_version()
            .domain(&domain)
            .repository(&repository)
            .format(PackageFormat::Pypi)
            .package(&package)
            .package_version(&seed.version);
        if !domain_owner.is_empty() {
            request = request.domain_owner(&domain_owner);
        }
        requests.push((index, seed.version, request));
    }

    let described = stream::iter(requests)
        .map(|(index, version, request)| async move {
            let (row, failed) = match request.send().await {
                Ok(response) => (
                    json!({
                        "version": version,
                        "published": dt_iso(
                            response
                                .package_version()
                                .and_then(|description| description.published_time())
                        ),
                    }),
                    false,
                ),
                Err(error) => (
                    json!({
                        "version": version,
                        "published": "",
                        "error": err_msg(error),
                    }),
                    true,
                ),
            };
            (index, row, failed)
        })
        .buffer_unordered(VERSION_DETAIL_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    rows.extend(described);
    version_history(&package, rows)
}

fn version_history(package: &str, mut rows: Vec<(usize, Value, bool)>) -> Value {
    rows.sort_by_key(|(index, _, _)| *index);
    let failed_count = rows.iter().filter(|(_, _, failed)| *failed).count();
    let mut coverage = Coverage::unknown(rows.len());
    coverage.count("failed", failed_count);
    coverage.limit("versions", Some(RECENT_VERSION_LIMIT));
    coverage.unknown_reason("version_snapshot", "These dates describe the supplied recent-version snapshot, not the complete package history.");
    if failed_count > 0 {
        coverage.failure(
            "enrichment_failed",
            "Some version publication dates could not be loaded.",
            true,
        );
    }
    coverage.attach(with_partial_failure(
        json!({
            "render": "codeartifact_version_history",
            "package": package,
            "versions": rows.into_iter().map(|(_, row, _)| row).collect::<Vec<_>>(),
        }),
        failed_count,
    ))
}

/// Failure is an explicit SDK outcome, never inferred from resource text.
fn with_partial_failure(mut response: Value, failed_count: usize) -> Value {
    if failed_count > 0 {
        response["ok"] = json!(false);
        response["partial"] = json!(true);
        response["status"] = json!("partial");
        response["error_type"] = json!("PartialFailure");
        response["failed_count"] = json!(failed_count);
    }
    response
}

fn version_seeds(value: Option<&Value>) -> Vec<VersionSeed> {
    let Some(values) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut seeds = Vec::new();

    for value in values {
        let (version, published) = match value {
            Value::String(version) => (version.trim(), ""),
            Value::Object(object) => (
                object
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim(),
                object
                    .get("published")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim(),
            ),
            _ => ("", ""),
        };
        if version.is_empty() || !seen.insert(version.to_string()) {
            continue;
        }
        let published = if DateTime::from_str(published, Format::DateTime).is_ok() {
            published.to_string()
        } else {
            String::new()
        };
        seeds.push(VersionSeed {
            version: version.to_string(),
            published,
        });
        if seeds.len() == RECENT_VERSION_LIMIT {
            break;
        }
    }

    seeds
}

fn version_history_error(package: &str, error: String) -> Value {
    json!({
        "render": "codeartifact_version_history",
        "package": package,
        "versions": [],
        "error": error,
    })
}

async fn list_packages(
    ctx: &WidgetCtx,
    client: &Client,
    domain: &str,
    repository: &str,
    package_prefix: &str,
    domain_owner: &str,
    max_packages: usize,
) -> Result<PackageListing, Value> {
    let mut packages = Vec::new();
    let mut next_token: Option<String> = None;
    let mut pages = 0usize;
    let mut error = None;
    let mut denied_page = false;

    while packages.len() < max_packages {
        if let Some(denied) = ctx.preflight("codeartifact", "ListPackages") {
            if pages == 0 {
                return Err(denied);
            }
            denied_page = true;
            error = Some("Additional package pages were not permitted.".to_string());
            break;
        }

        let page_size = (max_packages - packages.len()).min(1000) as i32;
        let mut req = client
            .list_packages()
            .domain(domain)
            .repository(repository)
            .format(PackageFormat::Pypi)
            .package_prefix(package_prefix)
            .max_results(page_size);
        if !domain_owner.is_empty() {
            req = req.domain_owner(domain_owner);
        }
        if let Some(token) = next_token.take() {
            req = req.next_token(token);
        }

        let resp = match req.send().await {
            Ok(response) => response,
            Err(failure) => {
                error = Some(err_msg(failure));
                break;
            }
        };
        pages += 1;
        packages.extend(
            resp.packages()
                .iter()
                .filter_map(|summary| summary.package().map(str::to_string)),
        );

        next_token = resp
            .next_token()
            .filter(|token| !token.is_empty())
            .map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }

    let limited =
        packages.len() > max_packages || (packages.len() >= max_packages && next_token.is_some());
    packages.truncate(max_packages);
    let mut coverage = Coverage::complete(packages.len());
    coverage.count("pages", pages);
    coverage.limit("results", Some(max_packages));
    if limited {
        coverage.has_more(Some(true));
        coverage.limited(
            "result_limit",
            "Additional matching packages were not loaded after the requested limit.",
        );
    }
    if error.is_some() {
        coverage.failure(
            if denied_page {
                "policy_denied"
            } else {
                "request_failed"
            },
            "A package-list page could not be loaded; earlier package names are retained.",
            pages > 0,
        );
    }
    Ok(PackageListing {
        packages,
        coverage,
        error,
    })
}

async fn latest_package_row(
    ctx: &WidgetCtx,
    client: &Client,
    domain: &str,
    repository: &str,
    package: String,
    domain_owner: &str,
) -> Result<PackageRow, Value> {
    if let Some(denied) = ctx.preflight("codeartifact", "ListPackageVersions") {
        return Err(denied);
    }

    let mut versions_req = client
        .list_package_versions()
        .domain(domain)
        .repository(repository)
        .format(PackageFormat::Pypi)
        .package(&package)
        .status(PackageVersionStatus::Published)
        .sort_by(PackageVersionSortType::PublishedTime)
        .max_results(RECENT_VERSION_LIMIT as i32);
    if !domain_owner.is_empty() {
        versions_req = versions_req.domain_owner(domain_owner);
    }

    let versions = match versions_req.send().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(PackageRow {
                package,
                latest_version: String::new(),
                last_published: format!("error: {}", err_msg(e)),
                versions: Vec::new(),
                detail_failed: true,
                detail_denied: false,
                versions_limited: false,
            });
        }
    };

    let recent_versions =
        recent_version_strings(versions.versions(), versions.default_display_version());
    let versions_limited = versions.versions().len() > RECENT_VERSION_LIMIT
        || versions.next_token().is_some_and(|token| !token.is_empty());
    let latest_version = recent_versions.first().cloned().unwrap_or_default();

    if latest_version.is_empty() {
        return Ok(PackageRow {
            package,
            latest_version,
            last_published: String::new(),
            versions: recent_versions,
            detail_failed: false,
            detail_denied: false,
            versions_limited,
        });
    }

    if ctx
        .preflight("codeartifact", "DescribePackageVersion")
        .is_some()
    {
        return Ok(PackageRow {
            package,
            latest_version,
            last_published: String::new(),
            versions: recent_versions,
            detail_failed: true,
            detail_denied: true,
            versions_limited,
        });
    }

    let mut describe_req = client
        .describe_package_version()
        .domain(domain)
        .repository(repository)
        .format(PackageFormat::Pypi)
        .package(&package)
        .package_version(&latest_version);
    if !domain_owner.is_empty() {
        describe_req = describe_req.domain_owner(domain_owner);
    }

    let described = match describe_req.send().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(PackageRow {
                package,
                latest_version,
                last_published: format!("error: {}", err_msg(e)),
                versions: recent_versions,
                detail_failed: true,
                detail_denied: false,
                versions_limited,
            });
        }
    };

    Ok(PackageRow {
        package,
        latest_version,
        last_published: dt_iso(described.package_version().and_then(|p| p.published_time())),
        versions: recent_versions,
        detail_failed: false,
        detail_denied: false,
        versions_limited,
    })
}

fn recent_version_strings(
    summaries: &[PackageVersionSummary],
    default_display_version: Option<&str>,
) -> Vec<String> {
    let mut versions: Vec<String> = summaries
        .iter()
        .map(|summary| summary.version().trim())
        .filter(|version| !version.is_empty())
        .take(RECENT_VERSION_LIMIT)
        .map(str::to_string)
        .collect();

    if versions.is_empty() {
        if let Some(version) = default_display_version
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            versions.push(version.to_string());
        }
    }

    versions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(version: &str) -> PackageVersionSummary {
        PackageVersionSummary::builder()
            .version(version)
            .status(PackageVersionStatus::Published)
            .build()
            .expect("valid package version summary")
    }

    #[test]
    fn recent_versions_keep_order_and_cap_at_ten() {
        let summaries: Vec<_> = (1..=12)
            .rev()
            .map(|version| summary(&format!("1.0.{version}")))
            .collect();

        let versions = recent_version_strings(&summaries, None);

        assert_eq!(versions.len(), RECENT_VERSION_LIMIT);
        assert_eq!(versions.first().map(String::as_str), Some("1.0.12"));
        assert_eq!(versions.last().map(String::as_str), Some("1.0.3"));
    }

    #[test]
    fn recent_versions_fall_back_to_default_display_version() {
        assert_eq!(
            recent_version_strings(&[], Some("2.4.0")),
            vec!["2.4.0".to_string()]
        );
    }

    #[test]
    fn version_history_input_is_deduplicated_validated_and_capped() {
        let mut values = vec![
            json!({"version": " 1.2.3 ", "published": "2026-07-01T09:36:37Z"}),
            json!({"version": "1.2.3", "published": "2026-07-01T09:36:37Z"}),
            json!({"version": "1.2.2", "published": "not-a-date"}),
            json!(""),
        ];
        values.extend((0..12).map(|index| json!(format!("1.1.{index}"))));

        let seeds = version_seeds(Some(&Value::Array(values)));

        assert_eq!(seeds.len(), RECENT_VERSION_LIMIT);
        assert_eq!(seeds[0].version, "1.2.3");
        assert_eq!(seeds[0].published, "2026-07-01T09:36:37Z");
        assert_eq!(seeds[1].version, "1.2.2");
        assert!(seeds[1].published.is_empty());
        assert_eq!(seeds[2].version, "1.1.0");
    }

    #[test]
    fn package_enrichment_failure_is_partial_without_scanning_row_text() {
        let row = |failed| PackageRow {
            package: "synthetic-package".into(),
            latest_version: "1.0.0".into(),
            last_published: "error: synthetic row text".into(),
            versions: vec!["1.0.0".into()],
            detail_failed: failed,
            detail_denied: false,
            versions_limited: false,
        };
        let successful = package_table(vec![row(false)]);
        assert_eq!(crate::request::outcome(&successful), "succeeded");
        assert!(successful.get("partial").is_none());
        let failed = package_table(vec![row(true)]);
        assert_eq!(crate::request::outcome(&failed), "failed");
        assert_eq!(failed["rows"], successful["rows"]);
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["partial"], true);
        assert_eq!(failed["status"], "partial");
        assert_eq!(failed["failed_count"], 1);
    }

    #[test]
    fn version_enrichment_failure_preserves_order_rows_and_partial_classification() {
        let known = json!({"version": "1.0.2", "published": "2026-01-01T00:00:00Z"});
        let unavailable = json!({"version": "1.0.1", "published": "", "error": "The AWS request failed. Check the connection and try again."});
        let result = version_history(
            "synthetic-package",
            vec![(1, unavailable.clone(), true), (0, known.clone(), false)],
        );
        assert_eq!(result["versions"], json!([known, unavailable]));
        assert_eq!(result["package"], "synthetic-package");
        assert_eq!(result["error_type"], "PartialFailure");
        assert_eq!(result["failed_count"], 1);
        assert_eq!(crate::request::outcome(&result), "failed");
        let complete = version_history("synthetic-package", vec![(0, known, false)]);
        assert_eq!(crate::request::outcome(&complete), "succeeded");
    }
}
