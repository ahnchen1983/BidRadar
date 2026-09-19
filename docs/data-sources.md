# Data Sources

## PCC Daily Gazette HTML

**Adapter name:** `PCC_DAILY_HTML`

**Official entry point:**

```text
GET https://web.pcc.gov.tw/prkms/tender/common/noticeDate/readPublish
    ?dateStr={ROC year}年{MM}月{DD}日
```

The page publishes all announcement categories for one date in a single HTML
document. BidRadar reads only the minimum list fields needed for first-stage
matching:

- notice category;
- agency name;
- tender number;
- tender title;
- publication date;
- PCC notice-version filename;
- official redirect URL.

No undocumented JSON API is assumed. All PCC-specific date conversion, HTML
parsing and URL construction live inside the adapter so another official source
can replace it without changing the rule engine or persistence layer.

## Included V0.1 Notice Categories

- Public tender notices and corrections
- Public-selection / public-solicitation restricted tender notices and corrections
- Selective tender notices and corrections
- Subsequent selective-tender invitations and corrections
- Public requests for vendor reference information and corrections
- Public tender-document previews and corrections
- Public requests for quotations or proposals and corrections

Award notices, failed-award notices, suspended-vendor lists, property sale
notices and property lease notices are not part of the V0.1 new-opportunity
scanner.

## Identity and Versioning

The daily list does not expose the canonical agency code. Until full detail is
fetched, the adapter derives a deterministic `pcc-index-v1` tender identity from
the normalized agency name and tender number. The PCC filename is stored as the
notice `version_key`.

This means:

- the same case can receive more than one published version;
- re-running the same day is idempotent;
- a correction is preserved rather than overwriting the original notice;
- full-detail ingestion can later reconcile the index identity with official
  agency and detail identifiers.

## Operational Guardrails

- Descriptive `User-Agent`
- 15-second connection timeout and configurable total timeout
- 16 MiB response-size ceiling
- One request per requested publication date
- No detail-page fan-out before a Watch Rule match
- Valid empty-day handling
- Explicit rejection of unexpected pages instead of treating them as zero data
- Recorded collector run status and counts

Parser tests use a small checked-in fixture that includes a normal notice, a
correction and an excluded award notice. Live source availability is not
required for CI unit tests.
