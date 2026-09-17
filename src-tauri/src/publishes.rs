//! 发布记录的持久化。
//!
//! 背景：发布结果原本只存在于弹窗里，关掉就再也找不回 URL / 认领链接。
//! 这里把每次发布落盘到 `~/.portbutler/publishes.json`，前端可随时列表查看。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::projects::data_dir;

/// 最多保留的历史条数，超出部分按时间从旧到新丢弃
const MAX_RECORDS: usize = 50;

/// 默认认领窗口（分钟）。wrangler 输出里通常会带实际值，取不到时用它兜底。
pub const DEFAULT_CLAIM_MINUTES: i64 = 60;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecord {
    /// worker 名（`pb-<毫秒时间戳>`），天然唯一，可直接当主键
    pub id: String,
    /// 关联项目 id（手动发布时可能为空）
    pub project_id: String,
    pub project_name: String,
    /// 本次发布的产物目录
    pub dist_path: String,
    pub url: String,
    /// 认领链接；为 None 表示非临时部署（已登录账号，长期有效）
    pub claim_url: Option<String>,
    /// 发布时刻（毫秒时间戳）
    pub published_at: i64,
    /// 链接失效时刻（毫秒时间戳）；None = 长期有效
    pub expires_at: Option<i64>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn store_path() -> PathBuf {
    data_dir().join("publishes.json")
}

pub fn load() -> Vec<PublishRecord> {
    fs::read_to_string(store_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(list: &[PublishRecord]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(store_path(), json).map_err(|e| e.to_string())
}

/// 新的在前，并裁掉超限的旧记录
fn sort_and_trim(list: &mut Vec<PublishRecord>) {
    list.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    list.truncate(MAX_RECORDS);
}

/// 追加一条记录（同 id 覆盖），返回写入后的完整列表。
/// 落盘失败不报错 —— 发布本身已经成功了，记录写不进去不该让用户看到失败。
pub fn push(rec: PublishRecord) -> Vec<PublishRecord> {
    let mut list = load();
    list.retain(|r| r.id != rec.id);
    list.push(rec);
    sort_and_trim(&mut list);
    let _ = save(&list);
    list
}

/// 从 wrangler 输出里解析 `Claim within: 18 minutes` 的分钟数。
/// 复用旧临时账号时这个值会小于 60，所以必须读实际值，不能写死。
pub fn parse_claim_minutes(out: &str) -> Option<i64> {
    const KEY: &str = "Claim within:";
    for line in out.lines() {
        let Some(idx) = line.find(KEY) else { continue };
        let rest = line[idx + KEY.len()..].trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<i64>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}

#[tauri::command]
pub fn list_publishes() -> Vec<PublishRecord> {
    let mut list = load();
    sort_and_trim(&mut list);
    list
}

/// 只删除本地记录，不动 Cloudflare 上的部署
#[tauri::command]
pub fn delete_publish(id: String) -> Vec<PublishRecord> {
    let mut list = load();
    list.retain(|r| r.id != id);
    let _ = save(&list);
    list
}

#[tauri::command]
pub fn clear_publishes() -> Vec<PublishRecord> {
    let _ = save(&[]);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, ts: i64) -> PublishRecord {
        PublishRecord {
            id: id.into(),
            project_id: "p1".into(),
            project_name: "演示".into(),
            dist_path: "/tmp/dist".into(),
            url: format!("https://{id}.demo.workers.dev"),
            claim_url: Some("https://dash.cloudflare.com/claim-preview?claimToken=x".into()),
            published_at: ts,
            expires_at: Some(ts + 3_600_000),
        }
    }

    #[test]
    fn claim_minutes_parsed_from_real_output() {
        let out = "Continuing means you accept Cloudflare's Terms of Service.\n\
                   Temporary account ready:\n\
                   \x20 Account: Mangrove Moth (reused)\n\
                   \x20 Claim within: 18 minutes\n\
                   \x20 Claim URL: https://dash.cloudflare.com/claim-preview?claimToken=abc\n\
                   Uploaded pb-1\nDeployed pb-1 triggers https://pb-1.x.workers.dev";
        assert_eq!(parse_claim_minutes(out), Some(18));
    }

    #[test]
    fn claim_minutes_absent_is_none() {
        assert_eq!(parse_claim_minutes("Deployed pb-1\nCurrent Version ID: 12"), None);
        // 有 key 但后面不是数字（不要让解析 panic 或误判）
        assert_eq!(parse_claim_minutes("Claim within: soon"), None);
    }

    #[test]
    fn sort_is_newest_first_and_trim_caps_length() {
        let mut list: Vec<PublishRecord> =
            (0..MAX_RECORDS as i64 + 5).map(|i| rec(&format!("pb-{i}"), i)).collect();
        sort_and_trim(&mut list);
        assert_eq!(list.len(), MAX_RECORDS);
        assert_eq!(list[0].id, format!("pb-{}", MAX_RECORDS as i64 + 4)); // 最新在最前
        assert!(list[0].published_at >= list[1].published_at);
        // 最旧的那几条被裁掉
        assert!(!list.iter().any(|r| r.id == "pb-0"));
    }

    #[test]
    fn record_roundtrips_through_json_as_camel_case() {
        let r = rec("pb-42", 1_700_000_000_000);
        let json = serde_json::to_string(&r).unwrap();
        // 前端按 camelCase 读，字段名不能是 snake_case
        assert!(json.contains("\"claimUrl\""), "json = {json}");
        assert!(json.contains("\"publishedAt\""), "json = {json}");
        assert!(json.contains("\"expiresAt\""), "json = {json}");
        let back: PublishRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "pb-42");
        assert_eq!(back.expires_at, Some(1_700_003_600_000));
    }

    #[test]
    fn expired_record_is_recognisable_from_expires_at() {
        // 前端靠 expiresAt 与当前时间比较来判定失效；这里保证 None 与 Some 都合法
        let mut r = rec("pb-1", 0);
        r.expires_at = None; // 长期有效（非临时部署）
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"expiresAt\":null"));
    }
}
