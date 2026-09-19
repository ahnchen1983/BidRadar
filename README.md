# BidRadar

BidRadar 是一套以「需求驅動（demand-driven）」為核心的政府標案雷達系統。

它不嘗試鏡像保存所有政府採購資料，而是：

1. 每日掃描新公告的最低必要欄位。
2. 依「機關單位」與「案件關鍵字」規則比對。
3. 未命中的公告不保存。
4. 命中的公告才寫入 BidRadar，並進一步取得完整資料。
5. 歷史搜尋採 Lazy Fetch；第一次實際查詢後才保存結果與查詢覆蓋範圍。
6. 已保存資料保留來源、抓取時間、版本與內容雜湊，以便日後重新核對。

## V0.1 Goal

目前目標是 **GOAL-001 — Data Pipeline Prototype**：

- Rust + Axum API
- PostgreSQL + SQLx
- Watch Rules：Agency / Tender Keyword
- Daily Collector 介面
- Rule Engine
- 命中才持久化
- Tender Version 基礎
- Notification Queue 基礎

詳細進度見 [GOALS.md](./GOALS.md)。

## 技術方向

- Backend: Rust / Axum / Tokio
- Database: PostgreSQL
- DB access: SQLx
- Frontend: Next.js（後續 Goal）
- Cache / Queue: 先以 PostgreSQL 實作，Redis 視實際負載再加入
- Deployment: Docker Compose

## 原則

**Scan broadly, store selectively, verify continuously.**

政府來源是 Source of Truth；BidRadar PostgreSQL 是服務使用者查詢與監控的高速資料層。

## V0.1 API

```text
GET  /health
GET  /api/v1/watch-rules
POST /api/v1/watch-rules
POST /api/v1/match/preview
POST /api/v1/collect/daily
```

手動執行指定日期的每日掃描：

```http
POST /api/v1/collect/daily
Content-Type: application/json

{
  "date": "2026-09-18"
}
```

Collector 只會保存至少命中一條啟用中 Watch Rule 的公告。重跑同一天時，
相同公告版本不會重複建立；若沒有啟用中的規則，則不會呼叫政府來源。

官方來源、收錄類型與識別策略見 [docs/data-sources.md](./docs/data-sources.md)。

## 每日排程

Docker Compose 預設啟用排程，依台灣時間執行：

```text
06:00
12:00
18:00
23:00
```

每一個排程時段會依序補掃「昨天 → 今天」。服務重新啟動時會補執行最近
一個尚未被認領的排程時段；多個 API instance 同時啟動時，PostgreSQL
只允許其中一個取得該時段。

可透過以下環境變數調整：

```text
DAILY_SCHEDULER_ENABLED=true
DAILY_SCHEDULER_TIMEZONE=Asia/Taipei
DAILY_SCHEDULER_TIMES=06:00,12:00,18:00,23:00
DAILY_SCHEDULER_LOOKBACK_DAYS=1
DAILY_SCHEDULER_CATCH_UP_ON_STARTUP=true
```

完整行為與失敗處理見 [docs/scheduler.md](./docs/scheduler.md)。
