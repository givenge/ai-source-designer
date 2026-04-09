use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::domain::SamplingResult;
use crate::state::AppPaths;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerRequest<'a> {
    id: u32,
    method: &'a str,
    params: WorkerParams<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerParams<'a> {
    target_url: &'a str,
    test_keyword: Option<&'a str>,
    output_dir: String,
    fixtures_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerResponse {
    id: u32,
    result: Option<SamplingResult>,
    error: Option<String>,
}

pub fn sample_site(
    paths: &AppPaths,
    task_id: &str,
    target_url: &str,
    test_keyword: Option<&str>,
) -> Result<SamplingResult> {
    let request = WorkerRequest {
        id: 1,
        method: "sampleSite",
        params: WorkerParams {
            target_url,
            test_keyword,
            output_dir: paths.task_dir(task_id).to_string_lossy().to_string(),
            fixtures_dir: paths.fixtures_dir.to_string_lossy().to_string(),
        },
    };

    let payload = serde_json::to_vec(&request)?;
    let mut child = Command::new("node")
        .arg(&paths.worker_script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to start worker with script {}",
                paths.worker_script.display()
            )
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open worker stdin"))?;
    stdin.write_all(&payload)?;
    drop(stdin);

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("worker failed: {stderr}"));
    }

    let raw = String::from_utf8(output.stdout)?;
    let response: WorkerResponse =
        serde_json::from_str(raw.trim()).context("failed to parse worker response")?;

    if response.id != 1 {
        return Err(anyhow!("worker response id mismatch"));
    }

    response
        .result
        .ok_or_else(|| anyhow!(response.error.unwrap_or_else(|| "worker returned no result".into())))
}
