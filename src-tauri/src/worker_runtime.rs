use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use crate::state::AppPaths;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeCheck {
    pub node_path: Option<String>,
    pub node_version: Option<String>,
    pub playwright_path: Option<String>,
    pub playwright_version: Option<String>,
    pub browser_version: Option<String>,
    pub error: Option<String>,
}

struct SystemRuntime {
    node: PathBuf,
    playwright: Option<PathBuf>,
    browsers: Option<PathBuf>,
}

fn managed_directory(paths: &AppPaths) -> PathBuf {
    paths.data_dir.join("runtime/playwright")
}

fn find_binary(name: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    directories
        .iter()
        .map(|directory| directory.join(&name))
        .find(|path| path.is_file())
}

fn system_node(paths: &AppPaths) -> Result<PathBuf> {
    let mut directories: Vec<_> =
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
    }
    if let Some(home) = paths.data_dir.parent() {
        directories.extend([
            home.join(".volta/bin"),
            home.join(".asdf/shims"),
            home.join(".local/share/mise/shims"),
        ]);
    }
    if let Some(node) = find_binary("node", &directories) {
        return Ok(node);
    }
    if let Some(home) = paths.data_dir.parent() {
        let mut versions: Vec<_> = fs::read_dir(home.join(".nvm/versions/node"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        versions.sort_by_key(|path| {
            path.file_name()
                .map(|name| {
                    name.to_string_lossy()
                        .trim_start_matches('v')
                        .split('.')
                        .filter_map(|part| part.parse::<u32>().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        versions.reverse();
        if let Some(node) = find_binary(
            "node",
            &versions
                .iter()
                .map(|path| path.join("bin"))
                .collect::<Vec<_>>(),
        ) {
            return Ok(node);
        }
    }
    bail!("未找到系统 Node.js，请安装 Node.js（含 npm）后重试。")
}

fn node_command(runtime: &SystemRuntime) -> Result<Command> {
    let mut command = Command::new(&runtime.node);
    let mut search_paths = vec![runtime
        .node
        .parent()
        .ok_or_else(|| anyhow!("Node.js 路径无效"))?
        .to_path_buf()];
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    command.env("PATH", env::join_paths(search_paths)?);
    command.env_remove("AISD_PLAYWRIGHT_MODULE");
    if let Some(module) = &runtime.playwright {
        command.env("AISD_PLAYWRIGHT_MODULE", module);
    }
    if let Some(browsers) = &runtime.browsers {
        command.env("PLAYWRIGHT_BROWSERS_PATH", browsers);
    }
    Ok(command)
}

fn system_runtime(paths: &AppPaths) -> Result<SystemRuntime> {
    let node = system_node(paths)?;
    let managed = managed_directory(paths);
    let module = managed.join("node_modules/playwright");
    if module.join("package.json").is_file() {
        return Ok(SystemRuntime {
            node,
            playwright: Some(module),
            browsers: Some(managed.join("browsers")),
        });
    }
    let global = node
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("lib/node_modules/playwright"));
    if let Some(module) = global.filter(|module| module.join("package.json").is_file()) {
        return Ok(SystemRuntime {
            node,
            playwright: Some(module),
            browsers: None,
        });
    }
    let mut runtime = SystemRuntime {
        node,
        playwright: None,
        browsers: None,
    };
    let output = node_command(&runtime)?
        .current_dir(
            paths
                .worker_script
                .parent()
                .ok_or_else(|| anyhow!("Worker 路径无效"))?,
        )
        .args([
            "-e",
            "process.stdout.write(require.resolve('playwright/package.json'))",
        ])
        .output()
        .context("系统 Node.js 无法启动")?;
    if output.status.success() {
        runtime.playwright = Path::new(String::from_utf8_lossy(&output.stdout).trim())
            .parent()
            .map(Path::to_path_buf);
    }
    Ok(runtime)
}

pub fn worker_command(paths: &AppPaths) -> Result<Command> {
    let mut command = node_command(&system_runtime(paths)?)?;
    command.arg(&paths.worker_script);
    Ok(command)
}

pub fn check_runtime(paths: &AppPaths) -> Result<RuntimeCheck> {
    if !paths.worker_script.is_file() {
        bail!("Playwright Worker 文件缺失");
    }
    let runtime = system_runtime(paths)?;
    check_runtime_with(paths, &runtime)
}

fn check_runtime_with(paths: &AppPaths, runtime: &SystemRuntime) -> Result<RuntimeCheck> {
    let output = node_command(&runtime)?
        .current_dir(paths.worker_script.parent().unwrap())
        .args([
            "-e",
            r#"
const result = { nodeVersion: process.version };
(async () => {
  try {
    const target = process.env.AISD_PLAYWRIGHT_MODULE || 'playwright';
    const { chromium } = require(target);
    result.playwrightVersion = require(require('path').join(target, 'package.json')).version;
    const browser = await chromium.launch({ headless: true, timeout: 6000 });
    try { result.browserVersion = browser.version(); } finally { await browser.close(); }
  } catch (error) { result.error = error.message; }
  process.stdout.write(JSON.stringify(result));
})();"#,
        ])
        .output()
        .context("系统 Node.js 检测失败")?;
    if !output.status.success() {
        bail!(
            "{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(600)
                .collect::<String>()
        );
    }
    let mut result: RuntimeCheck =
        serde_json::from_slice(&output.stdout).context("运行环境检测未返回有效结果")?;
    result.node_path = Some(runtime.node.to_string_lossy().into());
    result.playwright_path = runtime
        .playwright
        .as_ref()
        .map(|path| path.to_string_lossy().into());
    Ok(result)
}

fn npm_cli(node: &Path) -> Result<PathBuf> {
    let directory = node.parent().ok_or_else(|| anyhow!("Node.js 路径无效"))?;
    let executable = directory.join(if cfg!(windows) { "npm.cmd" } else { "npm" });
    let candidates = [
        fs::canonicalize(executable).unwrap_or_default(),
        directory.join("node_modules/npm/bin/npm-cli.js"),
        directory.join("../lib/node_modules/npm/bin/npm-cli.js"),
        PathBuf::from("/usr/share/nodejs/npm/bin/npm-cli.js"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file() && path.extension().is_some_and(|extension| extension == "js"))
        .ok_or_else(|| anyhow!("未找到系统 npm，请安装包含 npm 的 Node.js。"))
}

fn repair_log_path(paths: &AppPaths) -> PathBuf {
    paths.logs_dir.join("playwright-repair.log")
}

pub fn repair_log(paths: &AppPaths) -> String {
    let bytes = fs::read(repair_log_path(paths)).unwrap_or_default();
    String::from_utf8_lossy(&bytes)
        .chars()
        .rev()
        .take(12_000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn append_repair_log(paths: &AppPaths, message: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(repair_log_path(paths))?;
    writeln!(file, "{} {message}", chrono::Utc::now().to_rfc3339())?;
    Ok(())
}

fn run_logged_command(mut command: Command, log_path: &Path, timeout: Duration) -> Result<()> {
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    command
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("无法启动修复命令")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let log = fs::read_to_string(log_path).unwrap_or_default();
            let tail: String = log
                .chars()
                .rev()
                .take(1_000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            bail!("修复命令退出（{status}）：{tail}");
        }
        if started.elapsed() >= timeout {
            #[cfg(unix)]
            {
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{}", child.id())])
                    .status();
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("修复超时（{} 秒），请检查网络后重试。", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn remove_runtime_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn promote_runtime(staged: &Path, current: &Path) -> Result<PathBuf> {
    let backup = current.with_extension("previous");
    remove_runtime_path(&backup)?;
    let had_previous = fs::symlink_metadata(current).is_ok();
    if had_previous {
        fs::rename(current, &backup)?;
    }
    if let Err(error) = fs::rename(staged, current) {
        if had_previous {
            fs::rename(&backup, current).context("恢复原运行环境失败")?;
        }
        return Err(error).context("启用修复后的运行环境失败");
    }
    Ok(backup)
}

pub fn repair_runtime(paths: &AppPaths) -> Result<RuntimeCheck> {
    fs::create_dir_all(&paths.logs_dir)?;
    fs::write(repair_log_path(paths), "")?;
    let result = (|| {
        append_repair_log(paths, "检测系统 Node.js/npm")?;
        if !paths.worker_script.is_file() {
            bail!("Playwright Worker 文件缺失，请恢复应用文件后重试。");
        }
        let node = system_node(paths)?;
        let npm = npm_cli(&node)?;
        let current = managed_directory(paths);
        let directory = current.with_extension("repair");
        remove_runtime_path(&directory)?;
        fs::create_dir_all(&directory)?;
        let runtime = SystemRuntime {
            node,
            playwright: Some(directory.join("node_modules/playwright")),
            browsers: Some(directory.join("browsers")),
        };
        let package: serde_json::Value = serde_json::from_str(include_str!("../../package.json"))?;
        let version = package["devDependencies"]["playwright"]
            .as_str()
            .ok_or_else(|| anyhow!("缺少 Playwright 版本配置"))?;
        append_repair_log(paths, "安装 Playwright")?;
        let mut install = node_command(&runtime)?;
        install
            .arg(npm)
            .args(["install", "--prefix"])
            .arg(&directory)
            .args([
                "--no-audit",
                "--no-fund",
                "--ignore-scripts",
                "--save-exact",
                "--color=false",
            ])
            .arg(format!("playwright@{version}"));
        run_logged_command(install, &repair_log_path(paths), Duration::from_secs(600))?;
        append_repair_log(paths, "安装 Chromium")?;
        let mut browsers = node_command(&runtime)?;
        browsers
            .arg(directory.join("node_modules/playwright/cli.js"))
            .args(["install", "chromium"])
            .env("PLAYWRIGHT_DOWNLOAD_CONNECTION_TIMEOUT", "120000");
        run_logged_command(browsers, &repair_log_path(paths), Duration::from_secs(600))?;
        append_repair_log(paths, "验证 Chromium 启动")?;
        let checked = check_runtime_with(paths, &runtime)?;
        if let Some(error) = &checked.error {
            append_repair_log(paths, error)?;
            bail!("安装完成，但运行验证失败：{error}");
        }
        if checked.browser_version.is_none() {
            bail!("Chromium 运行验证失败");
        }
        let backup = promote_runtime(&directory, &current)?;
        let checked = check_runtime(paths);
        if checked.as_ref().map_or(true, |check| {
            check.error.is_some() || check.browser_version.is_none()
        }) {
            fs::rename(&current, &directory)?;
            if backup.exists() {
                fs::rename(&backup, &current)?;
            }
            bail!(
                "启用后的运行验证失败，原运行环境已恢复：{}",
                checked.as_ref().map_or_else(
                    |error| error.to_string(),
                    |check| check
                        .error
                        .clone()
                        .unwrap_or_else(|| "Chromium 启动失败".into())
                )
            );
        }
        if let Err(error) = remove_runtime_path(&backup) {
            append_repair_log(paths, &format!("旧缓存清理失败：{error}"))?;
        }
        append_repair_log(paths, "修复完成")?;
        checked
    })();
    if let Err(error) = &result {
        let _ = append_repair_log(paths, &error.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> AppPaths {
        let root = env::temp_dir().join(format!("aisd-system-runtime-{}", uuid::Uuid::new_v4()));
        AppPaths {
            data_dir: root.join("data"),
            db_path: root.join("db"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("bundled/worker/index.mjs"),
        }
    }

    #[test]
    fn system_node_ignores_a_bundled_executable() {
        let paths = test_paths();
        let bundled = paths
            .worker_script
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(if cfg!(windows) { "node.exe" } else { "node" });
        fs::create_dir_all(paths.worker_script.parent().unwrap()).unwrap();
        fs::write(&bundled, "unused bundled node").unwrap();
        if let Ok(command) = worker_command(&paths) {
            assert_ne!(command.get_program(), bundled.as_os_str());
            assert_eq!(
                command.get_args().next(),
                Some(paths.worker_script.as_os_str())
            );
        } else {
            assert!(system_node(&paths).is_err());
        }
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn managed_runtime_sets_its_module_and_browser_paths() {
        let runtime = SystemRuntime {
            node: PathBuf::from("/system/bin/node"),
            playwright: Some(PathBuf::from("/user/runtime/node_modules/playwright")),
            browsers: Some(PathBuf::from("/user/runtime/browsers")),
        };
        let command = node_command(&runtime).unwrap();
        assert_eq!(command.get_program(), runtime.node.as_os_str());
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "AISD_PLAYWRIGHT_MODULE"
                && value == runtime.playwright.as_deref().map(Path::as_os_str)));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "PLAYWRIGHT_BROWSERS_PATH"
                && value == runtime.browsers.as_deref().map(Path::as_os_str)));
    }

    #[test]
    fn failed_promotion_restores_the_previous_environment() {
        let paths = test_paths();
        let current = managed_directory(&paths);
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("existing-runtime"), "keep me").unwrap();
        assert!(promote_runtime(&current.with_extension("missing"), &current).is_err());
        assert_eq!(
            fs::read_to_string(current.join("existing-runtime")).unwrap(),
            "keep me"
        );
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).unwrap();
    }

    #[test]
    #[ignore = "downloads Playwright and Chromium into a temporary directory"]
    fn repair_runtime_download_smoke() {
        let mut paths = test_paths();
        paths.worker_script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../playwright/worker/index.mjs");
        println!(
            "runtime repair smoke evidence: {}",
            paths.data_dir.display()
        );
        let checked = repair_runtime(&paths).unwrap();
        assert!(checked.error.is_none());
        assert!(checked.browser_version.is_some());
        assert!(checked
            .playwright_path
            .unwrap()
            .contains("runtime/playwright/node_modules"));
        assert!(repair_log(&paths).contains("修复完成"));
        let mut regression = node_command(&system_runtime(&paths).unwrap()).unwrap();
        regression.arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/worker-regression.mjs"),
        );
        assert!(regression.status().unwrap().success());
        println!(
            "system Node: {:?}; Playwright: {:?}; Chromium: {:?}",
            checked.node_version, checked.playwright_version, checked.browser_version
        );
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn installer_captures_large_output_reports_failure_and_stops_on_timeout() {
        let paths = test_paths();
        fs::create_dir_all(&paths.logs_dir).unwrap();
        let mut failed = Command::new("/bin/sh");
        failed.args(["-c", "i=0; while [ $i -lt 10000 ]; do echo install-output; i=$((i+1)); done; echo install-failed >&2; exit 2"]);
        let error = run_logged_command(failed, &repair_log_path(&paths), Duration::from_secs(5))
            .unwrap_err()
            .to_string();
        assert!(error.contains("install-failed"));
        assert!(repair_log(&paths).contains("install-failed"));
        let mut hung = Command::new("/bin/sh");
        hung.args(["-c", "sleep 10"]);
        let started = Instant::now();
        assert!(
            run_logged_command(hung, &repair_log_path(&paths), Duration::from_millis(100))
                .unwrap_err()
                .to_string()
                .contains("超时")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        fs::remove_dir_all(paths.data_dir.parent().unwrap()).unwrap();
    }
}
