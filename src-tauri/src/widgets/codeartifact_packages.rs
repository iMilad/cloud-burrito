//! CodeArtifact Packages - list PyPI packages matching a prefix and show the
//! latest published version for each package.

use aws_sdk_codeartifact::{
    types::{PackageFormat, PackageVersionSortType, PackageVersionStatus},
    Client,
};
use serde_json::{json, Value};

use super::{dt_iso, err_msg, WidgetCtx};

const DEFAULT_DOMAIN: &str = "example-domain";
const DEFAULT_REPOSITORY: &str = "example_pypi_repo";
const DEFAULT_PACKAGE_PREFIX: &str = "example";
const DEFAULT_MAX_PACKAGES: i64 = 50;
const MAX_PACKAGES: usize = 1000;

#[derive(Debug)]
struct PackageRow {
    package: String,
    latest_version: String,
    last_published: String,
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
        .max_results(1);
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
            });
        }
    };

    let latest_version = versions
        .versions()
        .first()
        .map(|summary| summary.version().to_string())
        .or_else(|| versions.default_display_version().map(str::to_string))
        .unwrap_or_default();

    if latest_version.is_empty() {
        return Ok(PackageRow {
            package,
            latest_version,
            last_published: String::new(),
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
            });
        }
    };

    Ok(PackageRow {
        package,
        latest_version,
        last_published: dt_iso(described.package_version().and_then(|p| p.published_time())),
    })
}

fn table_with_error(error: String) -> Value {
    json!({
        "render": "table",
        "columns": ["package", "latest_version", "last_published"],
        "rows": [],
        "error": error,
    })
}
