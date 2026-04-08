// src/pg/infer.rs
//
// Heuristic pattern matching for column descriptions.
//
// A pure function `infer_column_description(col_name, col_type) -> Option<String>`
// that applies ~200 patterns mapping column name + type to plain-language
// descriptions. Used by describe_table when a column has no COMMENT set.
//
// Pattern matching is done entirely with str methods (ends_with, starts_with,
// contains, eq) — no regex dependency. The function is intentionally simple
// and fast (no heap allocation beyond the returned String).
//
// Pattern categories:
//   1.  Primary key conventions (id, uuid, oid, pk)
//   2.  Foreign key conventions (*_id, *_uuid, *_fk)
//   3.  Timestamps (*_at, *_on, created, updated, deleted_at, etc.)
//   4.  Booleans (is_*, has_*, can_*, allow_*, enabled, active)
//   5.  Monetary (*_cents, *_amount, *_price, *_cost, *_fee, *_total)
//   6.  Contact / identity (email, phone, url, slug, handle, username)
//   7.  Counters / aggregates (*_count, *_total, *_num, *_qty, *_sum)
//   8.  JSON / metadata (metadata, settings, config, data, payload)
//   9.  Arrays (*_ids, *_tags, *_list, columns with [] type)
//  10.  Size / measurement (*_size, *_bytes, *_kb, *_mb, *_length)
//  11.  Version / rank (version, rank, position, priority, order_num)
//  12.  Geographic (latitude, longitude, country_code, postal_code)
//  13.  Auth / security (password, *_hash, *_token, api_key)
//  14.  Status / state (status, state, stage, phase, step)
//  15.  Name / description (name, title, description, body, content)
//
// Design: check most specific patterns first (exact match > suffix > prefix).
// Return None when no pattern applies (column gets no inferred description).

/// Infer a plain-language description for a column based on its name and type.
///
/// Returns `Some(description)` when a pattern matches, `None` otherwise.
///
/// The description is written to be readable by both humans and AI agents.
///
/// # Examples
///
/// ```
/// use pgmcp::pg::infer::infer_column_description;
///
/// assert_eq!(
///     infer_column_description("user_id", "integer"),
///     Some("Foreign key reference to the users table".to_string())
/// );
/// assert_eq!(
///     infer_column_description("created_at", "timestamp with time zone"),
///     Some("Timestamp when the record was created".to_string())
/// );
/// assert!(infer_column_description("xyzzy", "bytea").is_none());
/// ```
pub fn infer_column_description(col_name: &str, col_type: &str) -> Option<String> {
    let name = col_name.to_lowercase();
    let type_lc = col_type.to_lowercase();
    let is_array = type_lc.ends_with("[]") || type_lc.starts_with("array");
    let is_bool = type_lc == "boolean" || type_lc == "bool";
    let is_int = type_lc.contains("int") || type_lc == "serial" || type_lc == "bigserial";
    let is_uuid = type_lc.contains("uuid");
    let is_numeric =
        type_lc.contains("numeric") || type_lc.contains("decimal") || type_lc.contains("float");
    let is_text =
        type_lc.contains("text") || type_lc.contains("char") || type_lc.contains("varchar");
    let is_timestamp = type_lc.contains("timestamp") || type_lc == "date" || type_lc == "time";
    let is_json = type_lc.contains("json");

    // ── Category 1: Primary key ───────────────────────────────────────────────

    if name == "id" {
        return if is_uuid {
            Some("Primary key (UUID)".to_string())
        } else {
            Some("Primary key (auto-incrementing integer)".to_string())
        };
    }
    if name == "uuid" && (is_uuid || is_text) {
        return Some("Unique identifier (UUID)".to_string());
    }
    if name == "oid" {
        return Some("Postgres internal object identifier".to_string());
    }
    if name == "pk" {
        return Some("Primary key".to_string());
    }
    if name == "rowid" {
        return Some("Row identifier".to_string());
    }

    // ── Category 2: Foreign key conventions ──────────────────────────────────

    if let Some(prefix) = name.strip_suffix("_id")
        && !prefix.is_empty()
        && is_int
    {
        let table = pluralize_guess(prefix);
        return Some(format!("Foreign key reference to the {table} table"));
    }
    if let Some(prefix) = name.strip_suffix("_uuid")
        && !prefix.is_empty()
        && (is_uuid || is_text)
    {
        let table = pluralize_guess(prefix);
        return Some(format!("UUID foreign key reference to the {table} table"));
    }
    if let Some(prefix) = name.strip_suffix("_fk")
        && !prefix.is_empty()
    {
        let table = pluralize_guess(prefix);
        return Some(format!("Foreign key reference to the {table} table"));
    }
    // Plural *_ids → array of FK references
    if let Some(prefix) = name.strip_suffix("_ids")
        && !prefix.is_empty()
    {
        let table = pluralize_guess(prefix);
        return Some(format!(
            "Array of foreign key references to the {table} table"
        ));
    }

    // ── Category 3: Timestamps ────────────────────────────────────────────────

    if name == "created_at" || name == "created_on" || name == "created" {
        return Some("Timestamp when the record was created".to_string());
    }
    if name == "updated_at" || name == "updated_on" || name == "modified_at" {
        return Some("Timestamp when the record was last updated".to_string());
    }
    if name == "deleted_at" || name == "removed_at" {
        return Some("Soft-delete timestamp; NULL means the record is active".to_string());
    }
    if name == "published_at" {
        return Some("Timestamp when the record was published".to_string());
    }
    if name == "archived_at" {
        return Some("Timestamp when the record was archived".to_string());
    }
    if name == "expires_at" || name == "expiry_at" || name == "expiration_at" {
        return Some("Timestamp when the record expires".to_string());
    }
    if name == "started_at" || name == "start_at" {
        return Some("Timestamp when the operation started".to_string());
    }
    if name == "ended_at" || name == "end_at" || name == "completed_at" {
        return Some("Timestamp when the operation completed".to_string());
    }
    if name == "processed_at" {
        return Some("Timestamp when the record was processed".to_string());
    }
    if name == "confirmed_at" {
        return Some("Timestamp when the record was confirmed".to_string());
    }
    if name == "cancelled_at" || name == "canceled_at" {
        return Some("Timestamp when the record was cancelled".to_string());
    }
    if name == "scheduled_at" {
        return Some("Timestamp when the record is scheduled to be processed".to_string());
    }
    if name == "sent_at" || name == "dispatched_at" {
        return Some("Timestamp when the record was sent or dispatched".to_string());
    }
    if name == "last_login_at" || name == "last_seen_at" || name == "last_active_at" {
        return Some("Timestamp of the most recent user activity".to_string());
    }
    if name == "born_at" || name == "birth_date" || name == "date_of_birth" || name == "dob" {
        return Some("Date of birth".to_string());
    }
    // Generic *_at timestamp
    if let Some(prefix) = name.strip_suffix("_at")
        && !prefix.is_empty()
        && is_timestamp
    {
        let event = prefix.replace('_', " ");
        return Some(format!("Timestamp of the {event} event"));
    }
    // Generic *_on timestamp
    if let Some(prefix) = name.strip_suffix("_on")
        && !prefix.is_empty()
        && is_timestamp
    {
        let event = prefix.replace('_', " ");
        return Some(format!("Date or timestamp of the {event} event"));
    }
    if name == "timestamp" || name == "ts" {
        return Some("Event timestamp".to_string());
    }

    // ── Category 4: Booleans ─────────────────────────────────────────────────

    if is_bool {
        if name == "active" || name == "is_active" {
            return Some("Whether this record is currently active".to_string());
        }
        if name == "enabled" || name == "is_enabled" {
            return Some("Whether this feature or record is enabled".to_string());
        }
        if name == "deleted" || name == "is_deleted" {
            return Some("Soft-delete flag; true means the record has been removed".to_string());
        }
        if name == "archived" || name == "is_archived" {
            return Some("Whether this record has been archived".to_string());
        }
        if name == "verified" || name == "is_verified" {
            return Some("Whether this record has been verified".to_string());
        }
        if name == "published" || name == "is_published" {
            return Some("Whether this record is publicly visible".to_string());
        }
        if name == "locked" || name == "is_locked" {
            return Some("Whether this record is locked from modification".to_string());
        }
        if name == "flagged" || name == "is_flagged" {
            return Some("Whether this record has been flagged for review".to_string());
        }
        if let Some(suffix) = name.strip_prefix("is_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("has_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether this record has {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("can_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether the entity can {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("allow_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether to allow {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("enable_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether to enable {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("should_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether the system should {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("use_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether to use {desc}"));
        }
        if let Some(suffix) = name.strip_prefix("requires_") {
            let desc = suffix.replace('_', " ");
            return Some(format!("Whether {desc} is required"));
        }
    }

    // ── Category 5: Monetary ─────────────────────────────────────────────────

    if name.ends_with("_cents") || name == "amount_in_cents" {
        return Some("Monetary amount in cents (divide by 100 for the dollar value)".to_string());
    }
    if name.ends_with("_amount") {
        return Some("Monetary amount".to_string());
    }
    if name.ends_with("_price") || name == "price" {
        return Some("Price value".to_string());
    }
    if name.ends_with("_cost") || name == "cost" {
        return Some("Cost value".to_string());
    }
    if name.ends_with("_fee") || name == "fee" {
        return Some("Fee amount".to_string());
    }
    if name.ends_with("_discount") || name == "discount" {
        return Some("Discount amount or percentage".to_string());
    }
    if name.ends_with("_tax") || name == "tax" {
        return Some("Tax amount".to_string());
    }
    if name == "subtotal" || name == "sub_total" {
        return Some("Subtotal before taxes and fees".to_string());
    }
    if name == "grand_total" || name == "total_amount" {
        return Some("Grand total including all fees and taxes".to_string());
    }
    if name == "balance" || name == "account_balance" {
        return Some("Current account balance".to_string());
    }
    if name == "credit" || name == "credit_amount" {
        return Some("Credit amount".to_string());
    }
    if name == "debit" || name == "debit_amount" {
        return Some("Debit amount".to_string());
    }

    // ── Category 6: Contact / identity ───────────────────────────────────────

    if name == "email" || name == "email_address" || name.ends_with("_email") {
        return Some("Email address".to_string());
    }
    if name == "phone"
        || name == "phone_number"
        || name.ends_with("_phone")
        || name == "mobile"
        || name == "cell_phone"
    {
        return Some("Phone number".to_string());
    }
    if name == "url" || name.ends_with("_url") || name == "website" || name == "website_url" {
        return Some("URL".to_string());
    }
    if name == "slug" || name.ends_with("_slug") {
        return Some("URL-friendly identifier (slug)".to_string());
    }
    if name == "username" || name == "user_name" || name == "login" || name == "login_name" {
        return Some("Username for authentication".to_string());
    }
    if name == "handle" || name == "screen_name" || name == "display_name" {
        return Some("Public display name or handle".to_string());
    }
    if name == "avatar" || name == "avatar_url" || name == "profile_image" {
        return Some("URL or path to the avatar/profile image".to_string());
    }
    if name == "bio" || name == "biography" || name == "about" {
        return Some("Short biography or about text".to_string());
    }
    if name == "locale" || name == "language" || name == "lang" {
        return Some("Locale or language code (e.g. en, fr-CA)".to_string());
    }
    if name == "timezone" || name == "time_zone" || name == "tz" {
        return Some("Time zone identifier (e.g. America/New_York)".to_string());
    }

    // ── Category 7: Counters / aggregates ────────────────────────────────────

    if name.ends_with("_count") && is_int {
        let subject = name.trim_end_matches("_count").replace('_', " ");
        return Some(format!("Number of {subject}s"));
    }
    if name.ends_with("_total") && (is_int || is_numeric) {
        let subject = name.trim_end_matches("_total").replace('_', " ");
        return Some(format!("Total {subject}"));
    }
    if name.ends_with("_sum") && (is_int || is_numeric) {
        let subject = name.trim_end_matches("_sum").replace('_', " ");
        return Some(format!("Sum of {subject} values"));
    }
    if name.ends_with("_num") && is_int {
        let subject = name.trim_end_matches("_num").replace('_', " ");
        return Some(format!("Number or count of {subject}"));
    }
    if name.ends_with("_qty") && (is_int || is_numeric) {
        let subject = name.trim_end_matches("_qty").replace('_', " ");
        return Some(format!("Quantity of {subject}"));
    }
    if name.ends_with("_quantity") && (is_int || is_numeric) {
        let subject = name.trim_end_matches("_quantity").replace('_', " ");
        return Some(format!("Quantity of {subject}"));
    }
    if name == "retries" || name == "retry_count" || name == "attempt_count" {
        return Some("Number of retry attempts".to_string());
    }
    if name == "views" || name == "view_count" || name == "page_views" {
        return Some("Number of times viewed".to_string());
    }
    if name == "likes" || name == "like_count" {
        return Some("Number of likes".to_string());
    }
    if name == "shares" || name == "share_count" {
        return Some("Number of shares".to_string());
    }
    if name == "downloads" || name == "download_count" {
        return Some("Number of downloads".to_string());
    }
    if name == "clicks" || name == "click_count" {
        return Some("Number of clicks".to_string());
    }

    // ── Category 8: JSON / metadata ───────────────────────────────────────────

    if is_json {
        if name == "metadata" || name == "meta" {
            return Some("Arbitrary metadata stored as JSON".to_string());
        }
        if name == "settings" || name == "preferences" || name == "prefs" {
            return Some("User or entity settings stored as JSON".to_string());
        }
        if name == "config" || name == "configuration" {
            return Some("Configuration stored as JSON".to_string());
        }
        if name == "options" || name == "opts" {
            return Some("Options or flags stored as JSON".to_string());
        }
        if name == "properties" || name == "props" || name == "attributes" || name == "attrs" {
            return Some("Additional properties stored as JSON".to_string());
        }
        if name == "extra" || name == "extras" || name == "additional" {
            return Some("Extra data stored as JSON".to_string());
        }
        if name == "data" {
            return Some("Data payload stored as JSON".to_string());
        }
        if name == "payload" {
            return Some("Event or message payload stored as JSON".to_string());
        }
        if name == "context" || name == "ctx" {
            return Some("Context information stored as JSON".to_string());
        }
        if name == "tags" || name == "labels" {
            return Some("Tags or labels stored as JSON".to_string());
        }
        return Some("Data stored as JSON".to_string());
    }

    // ── Category 9: Arrays ────────────────────────────────────────────────────

    if is_array {
        if name == "tags" || name.ends_with("_tags") {
            return Some("Array of tags".to_string());
        }
        if name == "labels" || name.ends_with("_labels") {
            return Some("Array of labels".to_string());
        }
        if name == "categories" || name.ends_with("_categories") {
            return Some("Array of category values".to_string());
        }
        if name == "roles" || name.ends_with("_roles") {
            return Some("Array of role names".to_string());
        }
        if name == "permissions" || name.ends_with("_permissions") {
            return Some("Array of permission strings".to_string());
        }
        if name == "emails" || name.ends_with("_emails") {
            return Some("Array of email addresses".to_string());
        }
        if name == "urls" || name.ends_with("_urls") {
            return Some("Array of URLs".to_string());
        }
        return Some("Array of values".to_string());
    }

    // ── Category 10: Size / measurement ──────────────────────────────────────

    if name.ends_with("_size") || name == "size" || name == "file_size" {
        return Some("Size in bytes".to_string());
    }
    if name.ends_with("_bytes") || name == "bytes" {
        return Some("Size or length in bytes".to_string());
    }
    if name.ends_with("_kb") {
        return Some("Size in kilobytes".to_string());
    }
    if name.ends_with("_mb") {
        return Some("Size in megabytes".to_string());
    }
    if name.ends_with("_gb") {
        return Some("Size in gigabytes".to_string());
    }
    if name.ends_with("_length") || name == "length" {
        return Some("Length value".to_string());
    }
    if name.ends_with("_width") || name == "width" {
        return Some("Width value".to_string());
    }
    if name.ends_with("_height") || name == "height" {
        return Some("Height value".to_string());
    }
    if name.ends_with("_weight") || name == "weight" {
        return Some("Weight value".to_string());
    }
    if name.ends_with("_duration") || name == "duration" {
        return Some("Duration in seconds or milliseconds".to_string());
    }

    // ── Category 11: Version / rank ───────────────────────────────────────────

    if name == "version" || name.ends_with("_version") {
        return Some("Record or schema version number".to_string());
    }
    if name == "rank" || name == "ranking" {
        return Some("Sort rank or priority order".to_string());
    }
    if name == "position" || name == "sort_order" || name == "display_order" || name == "order_num"
    {
        return Some("Display position or sort order".to_string());
    }
    if name == "priority" {
        return Some("Priority level".to_string());
    }
    if name == "sequence" || name == "seq" || name == "seq_num" {
        return Some("Sequence number".to_string());
    }
    if name == "revision" || name == "rev" {
        return Some("Revision number".to_string());
    }

    // ── Category 12: Geographic ───────────────────────────────────────────────

    if name == "latitude" || name == "lat" {
        return Some("Geographic latitude in decimal degrees".to_string());
    }
    if name == "longitude" || name == "lng" || name == "lon" || name == "long" {
        return Some("Geographic longitude in decimal degrees".to_string());
    }
    if name == "altitude" || name == "elevation" {
        return Some("Elevation above sea level in meters".to_string());
    }
    if name == "country" || name == "country_name" {
        return Some("Country name".to_string());
    }
    if name == "country_code" {
        return Some("ISO country code (e.g. US, GB, FR)".to_string());
    }
    if name == "region" || name == "state" || name == "province" {
        return Some("State, province, or region".to_string());
    }
    if name == "city" || name == "city_name" {
        return Some("City name".to_string());
    }
    if name == "address" || name == "street_address" || name == "addr" {
        return Some("Street address".to_string());
    }
    if name == "zip" || name == "zip_code" || name == "postal_code" || name == "postcode" {
        return Some("Postal or ZIP code".to_string());
    }
    if name == "location" {
        return Some("Geographic location".to_string());
    }
    if name == "geom" || name == "geometry" || name == "geog" || name == "geography" {
        return Some("Geometric or geographic shape".to_string());
    }

    // ── Category 13: Auth / security ─────────────────────────────────────────

    if name == "password" || name == "passwd" {
        return Some("Password (should be hashed — never store plaintext)".to_string());
    }
    if name.ends_with("_hash") || name == "password_hash" || name == "passwd_hash" {
        return Some("Cryptographic hash (e.g. bcrypt, argon2)".to_string());
    }
    if name.ends_with("_salt") {
        return Some("Cryptographic salt for hashing".to_string());
    }
    if name == "api_key" || name == "api_token" || name.ends_with("_api_key") {
        return Some("API authentication key".to_string());
    }
    if name == "access_token" || name.ends_with("_access_token") {
        return Some("OAuth or session access token".to_string());
    }
    if name == "refresh_token" || name.ends_with("_refresh_token") {
        return Some("OAuth refresh token".to_string());
    }
    if name.ends_with("_token") || name == "token" {
        return Some("Authentication or verification token".to_string());
    }
    if name.ends_with("_secret") || name == "secret" || name == "client_secret" {
        return Some("Secret key or value (sensitive — do not expose)".to_string());
    }
    if name == "session_id" || name == "session_token" {
        return Some("Session identifier".to_string());
    }
    if name == "otp" || name == "one_time_password" {
        return Some("One-time password code".to_string());
    }
    if name == "verification_code" || name == "confirm_code" {
        return Some("Verification or confirmation code".to_string());
    }

    // ── Category 14: Status / state ───────────────────────────────────────────

    if name == "status" {
        return Some("Current status of the record".to_string());
    }
    if name == "state" {
        return Some("Current state in the workflow".to_string());
    }
    if name == "stage" {
        return Some("Current stage in the process".to_string());
    }
    if name == "phase" {
        return Some("Current phase in the lifecycle".to_string());
    }
    if name == "step" || name == "step_name" {
        return Some("Current step in the process".to_string());
    }
    if name == "workflow_status" || name == "job_status" || name == "task_status" {
        return Some("Status of the workflow job or task".to_string());
    }
    if name == "progress" || name == "completion_pct" {
        return Some("Progress percentage (0-100)".to_string());
    }
    if name == "error_code" || name == "err_code" {
        return Some("Machine-readable error code".to_string());
    }
    if name == "error_message" || name == "err_msg" || name == "failure_reason" {
        return Some("Human-readable error message or failure reason".to_string());
    }

    // ── Category 15: Name / description ──────────────────────────────────────

    if name == "name" {
        return Some("Display name".to_string());
    }
    if name == "title" {
        return Some("Title or heading".to_string());
    }
    if name == "label" {
        return Some("Short label or tag".to_string());
    }
    if name == "description" || name == "desc" {
        return Some("Human-readable description".to_string());
    }
    if name == "summary" {
        return Some("Brief summary".to_string());
    }
    if name == "body" || name == "content" || name == "text" {
        return Some("Main text content".to_string());
    }
    if name == "note" || name == "notes" {
        return Some("Free-text notes or annotations".to_string());
    }
    if name == "comment" || name == "comments" {
        return Some("Comments or additional remarks".to_string());
    }
    if name == "message" || name == "msg" {
        return Some("Message text".to_string());
    }
    if name == "subject" || name == "subject_line" {
        return Some("Subject or heading of the message".to_string());
    }
    if name == "excerpt" || name == "snippet" {
        return Some("Short excerpt or preview of the content".to_string());
    }
    if name == "caption" {
        return Some("Caption or subtitle".to_string());
    }
    if name == "first_name" || name == "fname" {
        return Some("First (given) name".to_string());
    }
    if name == "last_name" || name == "lname" || name == "surname" || name == "family_name" {
        return Some("Last (family) name".to_string());
    }
    if name == "full_name" {
        return Some("Full name (first and last combined)".to_string());
    }
    if name == "middle_name" || name == "middle_initial" {
        return Some("Middle name or initial".to_string());
    }

    // ── No pattern matched ────────────────────────────────────────────────────

    None
}

/// Best-effort pluralization for FK table name inference.
///
/// `user` → `users`, `category` → `categories`, `tax` → `taxes`
fn pluralize_guess(word: &str) -> String {
    if word.is_empty() {
        return word.to_string();
    }
    if word.ends_with('s') || word.ends_with("ss") {
        // Already looks plural (or ends in ss → add "es" logically)
        if word.ends_with("ss") {
            return format!("{word}es");
        }
        return word.to_string();
    }
    if word.ends_with('y') && word.len() > 1 {
        let consonants = "bcdfghjklmnpqrstvwxz";
        let prev_char = word.chars().nth(word.len() - 2).unwrap_or('a');
        if consonants.contains(prev_char) {
            return format!("{}ies", &word[..word.len() - 1]);
        }
    }
    if word.ends_with('x') || word.ends_with("ch") || word.ends_with("sh") {
        return format!("{word}es");
    }
    format!("{word}s")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn infer(name: &str, ty: &str) -> Option<String> {
        infer_column_description(name, ty)
    }

    fn assert_inferred(name: &str, ty: &str, expected_contains: &str) {
        let desc = infer(name, ty);
        assert!(
            desc.is_some(),
            "expected description for ({name}, {ty}) but got None"
        );
        let desc = desc.unwrap();
        assert!(
            desc.to_lowercase()
                .contains(&expected_contains.to_lowercase()),
            "description for ({name}, {ty}) should contain '{expected_contains}', got: '{desc}'"
        );
    }

    fn assert_none(name: &str, ty: &str) {
        let desc = infer(name, ty);
        assert!(
            desc.is_none(),
            "expected None for ({name}, {ty}) but got: {:?}",
            desc
        );
    }

    // ── Category 1: Primary key ───────────────────────────────────────────────

    #[test]
    fn pk_id_integer() {
        assert_inferred("id", "integer", "primary key");
    }

    #[test]
    fn pk_id_uuid() {
        assert_inferred("id", "uuid", "uuid");
    }

    #[test]
    fn pk_uuid_column() {
        assert_inferred("uuid", "uuid", "unique identifier");
    }

    #[test]
    fn pk_oid() {
        assert_inferred("oid", "oid", "object identifier");
    }

    // ── Category 2: Foreign keys ──────────────────────────────────────────────

    #[test]
    fn fk_user_id() {
        assert_inferred("user_id", "integer", "users table");
    }

    #[test]
    fn fk_account_id() {
        assert_inferred("account_id", "bigint", "accounts table");
    }

    #[test]
    fn fk_category_id() {
        assert_inferred("category_id", "integer", "categories table");
    }

    #[test]
    fn fk_user_uuid() {
        assert_inferred("user_uuid", "uuid", "users table");
    }

    #[test]
    fn fk_ids_array() {
        assert_inferred("tag_ids", "integer[]", "tags table");
    }

    // ── Category 3: Timestamps ────────────────────────────────────────────────

    #[test]
    fn ts_created_at() {
        assert_inferred("created_at", "timestamp with time zone", "created");
    }

    #[test]
    fn ts_updated_at() {
        assert_inferred("updated_at", "timestamptz", "updated");
    }

    #[test]
    fn ts_deleted_at() {
        assert_inferred("deleted_at", "timestamptz", "soft-delete");
    }

    #[test]
    fn ts_published_at() {
        assert_inferred("published_at", "timestamp", "published");
    }

    #[test]
    fn ts_expires_at() {
        assert_inferred("expires_at", "timestamptz", "expires");
    }

    #[test]
    fn ts_generic_suffix_at() {
        assert_inferred("shipped_at", "timestamp", "shipped");
    }

    #[test]
    fn ts_generic_suffix_on() {
        assert_inferred("confirmed_on", "date", "confirmed");
    }

    // ── Category 4: Booleans ─────────────────────────────────────────────────

    #[test]
    fn bool_is_active() {
        assert_inferred("is_active", "boolean", "active");
    }

    #[test]
    fn bool_is_deleted() {
        assert_inferred("is_deleted", "boolean", "removed");
    }

    #[test]
    fn bool_has_profile() {
        assert_inferred("has_profile", "boolean", "has profile");
    }

    #[test]
    fn bool_can_edit() {
        assert_inferred("can_edit", "boolean", "can edit");
    }

    #[test]
    fn bool_active() {
        assert_inferred("active", "boolean", "active");
    }

    #[test]
    fn bool_enabled() {
        assert_inferred("enabled", "boolean", "enabled");
    }

    #[test]
    fn bool_requires_verification() {
        assert_inferred("requires_verification", "boolean", "required");
    }

    // ── Category 5: Monetary ─────────────────────────────────────────────────

    #[test]
    fn money_price_cents() {
        assert_inferred("price_cents", "integer", "cents");
    }

    #[test]
    fn money_order_amount() {
        assert_inferred("order_amount", "numeric", "monetary amount");
    }

    #[test]
    fn money_unit_price() {
        assert_inferred("unit_price", "numeric", "price");
    }

    #[test]
    fn money_shipping_cost() {
        assert_inferred("shipping_cost", "numeric", "cost");
    }

    #[test]
    fn money_service_fee() {
        assert_inferred("service_fee", "numeric", "fee");
    }

    #[test]
    fn money_balance() {
        assert_inferred("balance", "numeric", "balance");
    }

    // ── Category 6: Contact ───────────────────────────────────────────────────

    #[test]
    fn contact_email() {
        assert_inferred("email", "text", "email");
    }

    #[test]
    fn contact_email_suffix() {
        assert_inferred("work_email", "varchar(255)", "email");
    }

    #[test]
    fn contact_phone() {
        assert_inferred("phone", "text", "phone");
    }

    #[test]
    fn contact_url() {
        assert_inferred("url", "text", "url");
    }

    #[test]
    fn contact_slug() {
        assert_inferred("slug", "text", "slug");
    }

    #[test]
    fn contact_username() {
        assert_inferred("username", "varchar(100)", "username");
    }

    // ── Category 7: Counters ──────────────────────────────────────────────────

    #[test]
    fn counter_view_count() {
        // view_count matches the _count suffix rule → "Number of views"
        assert_inferred("view_count", "integer", "view");
    }

    #[test]
    fn counter_retry_count() {
        assert_inferred("retry_count", "integer", "retry");
    }

    #[test]
    fn counter_order_total() {
        assert_inferred("order_total", "numeric", "total");
    }

    #[test]
    fn counter_revenue_sum() {
        assert_inferred("revenue_sum", "numeric", "revenue");
    }

    #[test]
    fn counter_item_qty() {
        assert_inferred("item_qty", "integer", "quantity");
    }

    // ── Category 8: JSON ──────────────────────────────────────────────────────

    #[test]
    fn json_metadata() {
        assert_inferred("metadata", "jsonb", "metadata");
    }

    #[test]
    fn json_settings() {
        assert_inferred("settings", "json", "settings");
    }

    #[test]
    fn json_config() {
        assert_inferred("config", "jsonb", "configuration");
    }

    #[test]
    fn json_payload() {
        assert_inferred("payload", "jsonb", "payload");
    }

    #[test]
    fn json_generic_fallback() {
        assert_inferred("custom_data", "json", "json");
    }

    // ── Category 9: Arrays ────────────────────────────────────────────────────

    #[test]
    fn array_tags() {
        assert_inferred("tags", "text[]", "tags");
    }

    #[test]
    fn array_roles() {
        assert_inferred("roles", "text[]", "role");
    }

    #[test]
    fn array_generic() {
        assert_inferred("filters", "integer[]", "array");
    }

    // ── Category 10: Size ─────────────────────────────────────────────────────

    #[test]
    fn size_file_size() {
        assert_inferred("file_size", "bigint", "bytes");
    }

    #[test]
    fn size_content_length() {
        assert_inferred("content_length", "integer", "length");
    }

    #[test]
    fn size_duration() {
        assert_inferred("duration", "integer", "duration");
    }

    // ── Category 11: Version / rank ───────────────────────────────────────────

    #[test]
    fn version_version() {
        assert_inferred("version", "integer", "version");
    }

    #[test]
    fn version_schema_version() {
        assert_inferred("schema_version", "integer", "version");
    }

    #[test]
    fn version_rank() {
        assert_inferred("rank", "integer", "rank");
    }

    #[test]
    fn version_priority() {
        assert_inferred("priority", "integer", "priority");
    }

    // ── Category 12: Geographic ───────────────────────────────────────────────

    #[test]
    fn geo_latitude() {
        assert_inferred("latitude", "double precision", "latitude");
    }

    #[test]
    fn geo_longitude() {
        assert_inferred("longitude", "double precision", "longitude");
    }

    #[test]
    fn geo_lat() {
        assert_inferred("lat", "numeric", "latitude");
    }

    #[test]
    fn geo_country_code() {
        assert_inferred("country_code", "char(2)", "country");
    }

    #[test]
    fn geo_postal_code() {
        assert_inferred("postal_code", "varchar(20)", "postal");
    }

    #[test]
    fn geo_city() {
        assert_inferred("city", "text", "city");
    }

    // ── Category 13: Auth / security ─────────────────────────────────────────

    #[test]
    fn auth_password_hash() {
        assert_inferred("password_hash", "text", "hash");
    }

    #[test]
    fn auth_api_key() {
        assert_inferred("api_key", "text", "api");
    }

    #[test]
    fn auth_access_token() {
        assert_inferred("access_token", "text", "access token");
    }

    #[test]
    fn auth_refresh_token() {
        assert_inferred("refresh_token", "text", "refresh token");
    }

    #[test]
    fn auth_session_id() {
        assert_inferred("session_id", "text", "session");
    }

    #[test]
    fn auth_secret() {
        assert_inferred("client_secret", "text", "secret");
    }

    // ── Category 14: Status / state ───────────────────────────────────────────

    #[test]
    fn status_status() {
        assert_inferred("status", "text", "status");
    }

    #[test]
    fn status_state() {
        assert_inferred("state", "text", "state");
    }

    #[test]
    fn status_stage() {
        assert_inferred("stage", "text", "stage");
    }

    #[test]
    fn status_error_message() {
        assert_inferred("error_message", "text", "error");
    }

    // ── Category 15: Name / description ──────────────────────────────────────

    #[test]
    fn name_name() {
        assert_inferred("name", "text", "name");
    }

    #[test]
    fn name_title() {
        assert_inferred("title", "text", "title");
    }

    #[test]
    fn name_description() {
        assert_inferred("description", "text", "description");
    }

    #[test]
    fn name_body() {
        assert_inferred("body", "text", "content");
    }

    #[test]
    fn name_first_name() {
        assert_inferred("first_name", "text", "first");
    }

    #[test]
    fn name_last_name() {
        assert_inferred("last_name", "text", "last");
    }

    #[test]
    fn name_full_name() {
        assert_inferred("full_name", "text", "full name");
    }

    // ── No-match cases ────────────────────────────────────────────────────────

    #[test]
    fn no_match_random_name() {
        assert_none("xyzzy", "text");
    }

    #[test]
    fn no_match_bytea() {
        assert_none("raw_data", "bytea");
    }

    // type-guarded: user_id with text type should NOT be a FK (type guard fails)
    #[test]
    fn no_fk_user_id_text_type() {
        // user_id with non-integer type should not get FK description
        let desc = infer("user_id", "text");
        // Could be None or something generic — just ensure it's not calling it a FK to users
        if let Some(d) = desc {
            assert!(
                !d.contains("Foreign key"),
                "text user_id should not be called a FK: {d}"
            );
        }
    }

    // ── Pluralization helper ──────────────────────────────────────────────────

    #[test]
    fn pluralize_regular() {
        assert_eq!(pluralize_guess("user"), "users");
    }

    #[test]
    fn pluralize_y_ending() {
        assert_eq!(pluralize_guess("category"), "categories");
    }

    #[test]
    fn pluralize_x_ending() {
        assert_eq!(pluralize_guess("tax"), "taxes");
    }

    #[test]
    fn pluralize_already_plural() {
        assert_eq!(pluralize_guess("users"), "users");
    }
}
