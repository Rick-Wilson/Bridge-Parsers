//! ACBL member data fetching and parsing

use std::collections::HashMap;

/// ACBL member masterpoint information
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub name: String,
    pub location: String,
    pub rank: String,
    pub points: f64,
    pub unit: String,
}

/// Club game result from ACBL Live for Clubs
#[derive(Debug, Clone)]
pub struct ClubGameResult {
    pub club_name: String,
    pub event_name: String,
    pub date: String,
    pub mp_limits: String,
    pub event_type: Option<String>,
    pub tables: Option<u32>,
    pub sections: Vec<SectionResult>,
    pub pbn_url: Option<String>,
    pub bws_url: Option<String>,
}

/// Section results (NS or EW)
#[derive(Debug, Clone)]
pub struct SectionResult {
    pub section: String,
    pub direction: String,  // "NS" or "EW"
    pub pairs: Vec<PairResult>,
}

/// Individual pair result
#[derive(Debug, Clone)]
pub struct PairResult {
    pub pair_number: u32,
    pub player1: String,
    pub player2: String,
    pub strat: String,
    pub overall_a: Option<u32>,
    pub overall_b: Option<u32>,
    pub overall_c: Option<u32>,
    pub section_a: Option<u32>,
    pub section_b: Option<u32>,
    pub section_c: Option<u32>,
    pub score: f64,
    pub percentage: f64,
    pub masterpoints: Option<String>,
}

/// Create an HTTP client with browser-like headers
fn create_browser_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Fetch a URL with browser-like headers
pub fn fetch_with_browser_headers(url: &str) -> Result<String, String> {
    let client = create_browser_client()?;

    let response = client.get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Accept-Encoding", "gzip, deflate, br")
        .header("Connection", "keep-alive")
        .header("Upgrade-Insecure-Requests", "1")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-User", "?1")
        .header("Cache-Control", "max-age=0")
        .send()
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP error: {} {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown")));
    }

    response.text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

/// Fetch and parse ACBL Live for Clubs game results
pub fn fetch_club_game_results(url: &str) -> Result<ClubGameResult, String> {
    let html = fetch_with_browser_headers(url)?;
    parse_club_game_html(&html)
}

/// Parse ACBL Live for Clubs HTML
fn parse_club_game_html(html: &str) -> Result<ClubGameResult, String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    // Extract event metadata
    let club_name = extract_text_by_selector(&document, "h1, .club-name, [class*='club']")
        .unwrap_or_default();

    let event_name = extract_text_by_selector(&document, "h2, .event-name, [class*='event']")
        .unwrap_or_default();

    // Look for date, MP limits, tables in the page
    let page_text = document.root_element().text().collect::<String>();

    let date = extract_date_from_text(&page_text).unwrap_or_default();
    let mp_limits = extract_mp_limits_from_text(&page_text).unwrap_or_default();
    let tables = extract_tables_from_text(&page_text);
    let event_type = extract_event_type_from_text(&page_text);

    // Extract PBN and BWS URLs
    let pbn_url = extract_file_url(&document, "pbn");
    let bws_url = extract_file_url(&document, "bws");

    // Parse section results
    let sections = parse_section_results(&document)?;

    Ok(ClubGameResult {
        club_name,
        event_name,
        date,
        mp_limits,
        event_type,
        tables,
        sections,
        pbn_url,
        bws_url,
    })
}

fn extract_text_by_selector(document: &scraper::Html, selector_str: &str) -> Option<String> {
    use scraper::Selector;

    if let Ok(selector) = Selector::parse(selector_str) {
        if let Some(element) = document.select(&selector).next() {
            let text: String = element.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn extract_date_from_text(text: &str) -> Option<String> {
    // Look for date patterns like "01/19/2026" or "January 19, 2026"
    let date_regex = regex::Regex::new(r"\d{2}/\d{2}/\d{4}").ok()?;
    date_regex.find(text).map(|m| m.as_str().to_string())
}

fn extract_mp_limits_from_text(text: &str) -> Option<String> {
    // Look for "MP Limits: None/1000/500" pattern
    if let Some(idx) = text.find("MP Limits:") {
        let start = idx + "MP Limits:".len();
        let remaining = &text[start..];
        let end = remaining.find('\n').unwrap_or(remaining.len()).min(50);
        return Some(remaining[..end].trim().to_string());
    }
    None
}

fn extract_tables_from_text(text: &str) -> Option<u32> {
    // Look for "Tables: 11" pattern
    if let Some(idx) = text.find("Tables:") {
        let start = idx + "Tables:".len();
        let remaining = &text[start..].trim();
        let num_str: String = remaining.chars().take_while(|c| c.is_ascii_digit()).collect();
        return num_str.parse().ok();
    }
    None
}

fn extract_event_type_from_text(text: &str) -> Option<String> {
    // Look for common event types
    let event_types = ["Unit Championship", "Club Championship", "Upgraded Club Championship",
                       "Charity", "STaC", "NAP", "GNT"];
    for event_type in event_types {
        if text.contains(event_type) {
            return Some(event_type.to_string());
        }
    }
    None
}

fn extract_file_url(document: &scraper::Html, file_type: &str) -> Option<String> {
    use scraper::Selector;

    if let Ok(selector) = Selector::parse("a") {
        for link in document.select(&selector) {
            if let Some(href) = link.value().attr("href") {
                let text = link.text().collect::<String>().to_lowercase();
                let href_lower = href.to_lowercase();

                if text.contains(file_type) || href_lower.contains(file_type) {
                    return Some(href.to_string());
                }
            }
        }
    }
    None
}

fn parse_section_results(document: &scraper::Html) -> Result<Vec<SectionResult>, String> {
    use scraper::Selector;

    let mut sections = Vec::new();

    // Look for tables with recap data
    let table_selector = Selector::parse("table")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let row_selector = Selector::parse("tbody tr, tr")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let cell_selector = Selector::parse("td")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    // Try to identify which section/direction each table represents
    // by looking at nearby headers
    let header_selector = Selector::parse("h3, h4, .section-header, caption")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let mut current_section = "A".to_string();
    let mut current_direction = "NS".to_string();

    for element in document.select(&Selector::parse("*").unwrap()) {
        // Check if this is a header that indicates section/direction
        let tag = element.value().name();
        if tag == "h3" || tag == "h4" || tag == "caption" {
            let text = element.text().collect::<String>();
            if text.contains("NS") {
                current_direction = "NS".to_string();
            } else if text.contains("EW") {
                current_direction = "EW".to_string();
            }
            if let Some(section) = extract_section_letter(&text) {
                current_section = section;
            }
        }

        // Check if this is a table with results
        if tag == "table" {
            let mut pairs = Vec::new();

            for row in element.select(&row_selector) {
                let cells: Vec<String> = row
                    .select(&cell_selector)
                    .map(|cell| cell.text().collect::<String>().trim().to_string())
                    .collect();

                // Look for rows that look like pair results
                // Typical format: Pair#, Names, Strat, Overall places, Section places, Score, %, MPs
                if let Some(pair_result) = parse_pair_result_row(&cells) {
                    pairs.push(pair_result);
                }
            }

            if !pairs.is_empty() {
                sections.push(SectionResult {
                    section: current_section.clone(),
                    direction: current_direction.clone(),
                    pairs,
                });
            }
        }
    }

    Ok(sections)
}

fn extract_section_letter(text: &str) -> Option<String> {
    // Look for "Section A", "Section B", etc.
    let text_upper = text.to_uppercase();
    if text_upper.contains("SECTION") {
        for letter in ['A', 'B', 'C', 'D', 'E', 'F'] {
            if text_upper.contains(&format!("SECTION {}", letter))
               || text_upper.contains(&format!("SECTION{}", letter)) {
                return Some(letter.to_string());
            }
        }
    }
    None
}

fn parse_pair_result_row(cells: &[String]) -> Option<PairResult> {
    // Need at least pair number, names, and some results
    if cells.len() < 5 {
        return None;
    }

    // First cell should be pair number
    let pair_number: u32 = cells[0].parse().ok()?;

    // Second cell should be names (Player1 - Player2)
    let names = &cells[1];
    let (player1, player2) = if names.contains(" - ") {
        let parts: Vec<&str> = names.splitn(2, " - ").collect();
        (parts.get(0).unwrap_or(&"").to_string(),
         parts.get(1).unwrap_or(&"").to_string())
    } else {
        (names.clone(), String::new())
    };

    // Look for percentage and score in remaining cells
    let mut score = 0.0;
    let mut percentage = 0.0;
    let mut masterpoints = None;
    let mut strat = String::new();

    for (i, cell) in cells.iter().enumerate().skip(2) {
        // Strat is usually a single letter: A, B, or C
        if cell.len() == 1 && ["A", "B", "C"].contains(&cell.as_str()) && strat.is_empty() {
            strat = cell.clone();
            continue;
        }

        // Percentage usually contains a decimal and is between 0-100
        if cell.contains('.') {
            if let Ok(val) = cell.parse::<f64>() {
                if val <= 100.0 && val > 0.0 && percentage == 0.0 {
                    percentage = val;
                } else if score == 0.0 {
                    score = val;
                }
            }
        }

        // Masterpoints usually contain "Black", "Silver", "Gold", "Red", "Platinum"
        if cell.contains("Black") || cell.contains("Silver") || cell.contains("Gold")
           || cell.contains("Red") || cell.contains("Platinum") {
            masterpoints = Some(cell.clone());
        }
    }

    // Only return if we have valid data
    if percentage > 0.0 || score > 0.0 {
        Some(PairResult {
            pair_number,
            player1,
            player2,
            strat,
            overall_a: None,  // Would need more sophisticated parsing
            overall_b: None,
            overall_c: None,
            section_a: None,
            section_b: None,
            section_c: None,
            score,
            percentage,
            masterpoints,
        })
    } else {
        None
    }
}

/// Fetch and parse ACBL member data from a District 21 style URL
/// Returns a HashMap keyed by ACBL member number (as string)
pub fn fetch_member_masterpoints(url: &str) -> Result<HashMap<String, MemberInfo>, String> {
    // Fetch the page
    let response = reqwest::blocking::get(url)
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    let body = response.text()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    parse_member_html(&body)
}

/// Parse member data from HTML content
/// The D21 page has a table with columns: Member, Location, Rank, Points, Unit
fn parse_member_html(html: &str) -> Result<HashMap<String, MemberInfo>, String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    // Try to find table rows - the D21 site uses DataTables
    let row_selector = Selector::parse("table tbody tr")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let cell_selector = Selector::parse("td")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let mut members = HashMap::new();

    for row in document.select(&row_selector) {
        let cells: Vec<String> = row
            .select(&cell_selector)
            .map(|cell| cell.text().collect::<String>().trim().to_string())
            .collect();

        // Expected: Member, Location, Rank, Points, Unit
        // Some tables might have ACBL# as first column
        if cells.len() >= 5 {
            // Try to extract ACBL number from the member name or a separate column
            // The D21 page shows "Name" but we need to match by ACBL number
            // Let's check if there's a link with the ACBL number
            let acbl_num = extract_acbl_number(&row);

            let (name, location, rank, points_str, unit) = if cells.len() == 5 {
                (&cells[0], &cells[1], &cells[2], &cells[3], &cells[4])
            } else if cells.len() >= 6 {
                // First column might be ACBL number
                (&cells[1], &cells[2], &cells[3], &cells[4], &cells[5])
            } else {
                continue;
            };

            let points = points_str
                .replace(",", "")
                .parse::<f64>()
                .unwrap_or(0.0);

            let info = MemberInfo {
                name: name.clone(),
                location: location.clone(),
                rank: rank.clone(),
                points,
                unit: unit.clone(),
            };

            // If we found an ACBL number, use it as key
            if let Some(num) = acbl_num {
                members.insert(num, info.clone());
            }

            // Also index by name for fallback matching
            members.insert(name.to_lowercase(), info);
        }
    }

    if members.is_empty() {
        // Try alternate parsing - maybe it's not a standard table
        // Look for any pattern of member data
        return parse_member_html_alternate(html);
    }

    Ok(members)
}

/// Try to extract ACBL number from a table row (might be in a link or data attribute)
fn extract_acbl_number(row: &scraper::ElementRef) -> Option<String> {
    use scraper::Selector;

    // Check for links that might contain the ACBL number
    if let Ok(link_selector) = Selector::parse("a") {
        for link in row.select(&link_selector) {
            if let Some(href) = link.value().attr("href") {
                // Look for patterns like /member/1234567 or ?acbl=1234567
                if let Some(num) = extract_number_from_url(href) {
                    return Some(num);
                }
            }
        }
    }

    // Check for data attributes
    if let Some(data_id) = row.value().attr("data-id") {
        if data_id.chars().all(|c| c.is_ascii_digit()) {
            return Some(data_id.to_string());
        }
    }

    None
}

/// Extract a 7-digit ACBL number from a URL
fn extract_number_from_url(url: &str) -> Option<String> {
    // Look for 7-digit numbers (typical ACBL member numbers)
    let digits: String = url.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 7 {
        // Return the last 7 digits (most likely the member number)
        Some(digits[digits.len().saturating_sub(7)..].to_string())
    } else if digits.len() >= 5 {
        // Some older numbers are shorter
        Some(digits)
    } else {
        None
    }
}

/// Alternate parsing for non-standard table formats
fn parse_member_html_alternate(html: &str) -> Result<HashMap<String, MemberInfo>, String> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let mut members = HashMap::new();

    // Try to find any table
    let table_selector = Selector::parse("table")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let row_selector = Selector::parse("tr")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    let cell_selector = Selector::parse("td, th")
        .map_err(|e| format!("Invalid selector: {:?}", e))?;

    for table in document.select(&table_selector) {
        for row in table.select(&row_selector) {
            let cells: Vec<String> = row
                .select(&cell_selector)
                .map(|cell| cell.text().collect::<String>().trim().to_string())
                .collect();

            // Look for rows with numeric points value
            if cells.len() >= 4 {
                // Try to identify which column has points (numeric with decimals)
                for (i, cell) in cells.iter().enumerate() {
                    if cell.contains('.') && cell.replace(",", "").parse::<f64>().is_ok() {
                        // This is likely the points column
                        let points = cell.replace(",", "").parse::<f64>().unwrap_or(0.0);

                        // Assume: name is before points, rank might be just before points
                        let name = if i > 0 { &cells[0] } else { continue };
                        let rank = if i > 1 { &cells[i - 1] } else { "" };
                        let location = if i > 2 { &cells[1] } else { "" };
                        let unit = if i + 1 < cells.len() { &cells[i + 1] } else { "" };

                        let info = MemberInfo {
                            name: name.clone(),
                            location: location.to_string(),
                            rank: rank.to_string(),
                            points,
                            unit: unit.to_string(),
                        };

                        // Index by lowercase name
                        members.insert(name.to_lowercase(), info);
                        break;
                    }
                }
            }
        }
    }

    Ok(members)
}

/// Look up a member by ACBL number or name
pub fn lookup_member<'a>(
    members: &'a HashMap<String, MemberInfo>,
    acbl_number: &str,
    name: Option<&str>,
) -> Option<&'a MemberInfo> {
    // Try ACBL number first (strip leading zeros for matching)
    let normalized_num = acbl_number.trim_start_matches('0');
    if let Some(info) = members.get(normalized_num) {
        return Some(info);
    }
    if let Some(info) = members.get(acbl_number) {
        return Some(info);
    }

    // Fall back to name matching
    if let Some(name) = name {
        let name_lower = name.to_lowercase();
        if let Some(info) = members.get(&name_lower) {
            return Some(info);
        }

        // Try partial name match (last name)
        let last_name = name.split_whitespace().last().unwrap_or("").to_lowercase();
        for (key, info) in members {
            if key.contains(&last_name) && !last_name.is_empty() {
                return Some(info);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_number_from_url() {
        assert_eq!(
            extract_number_from_url("/member/1234567"),
            Some("1234567".to_string())
        );
        assert_eq!(
            extract_number_from_url("?id=9876543"),
            Some("9876543".to_string())
        );
    }
}
