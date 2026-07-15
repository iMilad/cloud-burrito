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

    let packages = match list_packages(
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

    if packages.is_empty() {
        return json!({
            "render": "table",
            "columns": ["package", "latest_version", "last_published"],
            "rows": [],
        });
    }

    let mut out: Vec<Value> = Vec::with_capacity(packages.len());
    for package in packages {
        let row =
            match latest_package_row(ctx, &client, &domain, &repository, package, &domain_owner)
                .await
            {
                Ok(row) => row,
                Err(render) => return render,
            };
        out.push(json!({
            "package": row.package,
            "latest_version": row.latest_version,
            "last_published": row.last_published,
            "versions": row.versions,
        }));
    }
    out.sort_by_key(|row| {
        row.get("package")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase()
    });

    json!({
        "render": "table",
        "columns": ["package", "latest_version", "last_published"],
        "rows": out,
    })
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
    let mut rows: Vec<(usize, Value)> = Vec::with_capacity(seeds.len());
    let mut requests = Vec::new();

    for (index, seed) in seeds.into_iter().enumerate() {
        if !seed.published.is_empty() {
            rows.push((
                index,
                json!({"version": seed.version, "published": seed.published}),
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
            let row = match request.send().await {
                Ok(response) => json!({
                    "version": version,
                    "published": dt_iso(
                        response
                            .package_version()
                            .and_then(|description| description.published_time())
                    ),
                }),
                Err(error) => json!({
                    "version": version,
                    "published": "",
                    "error": err_msg(error),
                }),
            };
            (index, row)
        })
        .buffer_unordered(VERSION_DETAIL_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    rows.extend(described);
    rows.sort_by_key(|(index, _)| *index);

    json!({
        "render": "codeartifact_version_history",
        "package": package,
        "versions": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
    })
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
) -> Result<Vec<String>, Value> {
    let mut packages = Vec::new();
    let mut next_token: Option<String> = None;

    while packages.len() < max_packages {
        if let Some(denied) = ctx.preflight("codeartifact", "ListPackages") {
            return Err(denied);
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

        let resp = req.send().await.map_err(|e| table_with_error(err_msg(e)))?;
        packages.extend(
            resp.packages()
                .iter()
                .filter_map(|summary| summary.package().map(str::to_string)),
        );

        next_token = resp.next_token().map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }

    packages.truncate(max_packages);
    Ok(packages)
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
            });
        }
    };

    let recent_versions =
        recent_version_strings(versions.versions(), versions.default_display_version());
    let latest_version = recent_versions.first().cloned().unwrap_or_default();

    if latest_version.is_empty() {
        return Ok(PackageRow {
            package,
            latest_version,
            last_published: String::new(),
            versions: recent_versions,
        });
    }

    if let Some(denied) = ctx.preflight("codeartifact", "DescribePackageVersion") {
        return Err(denied);
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
            });
        }
    };

    Ok(PackageRow {
        package,
        latest_version,
        last_published: dt_iso(described.package_version().and_then(|p| p.published_time())),
        versions: recent_versions,
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

fn table_with_error(error: String) -> Value {
    json!({
        "render": "table",
        "columns": ["package", "latest_version", "last_published"],
        "rows": [],
        "error": error,
    })
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
}
