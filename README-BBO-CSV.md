# BBO CSV Cardplay Extraction & DD Analysis Tool

A CLI tool to help extract and analyze cardplay data from BBO (Bridge Base Online) hand records.

## Purpose

Bridge players facing accusations of online cheating often receive evidence in the form of thousands of TinyURLs linking to BBO hand records. While these URLs contain the complete cardplay data, accessing it requires manually clicking through each hand in the BBO viewer.

This tool automates the extraction of cardplay data and performs double-dummy analysis to compute the "cost" of each card played.

## Installation

```bash
cd Bridge-Parsers
cargo build --release
```

The `bbo-csv` binary will be available at `target/release/bbo-csv`.

## Usage

### Step 1: Extract Cardplay Data

```bash
bbo-csv fetch-cardplay \
  --input "Hand Records.csv" \
  --output "Hand Records with Cardplay.csv" \
  --url-column "BBO" \
  --delay-ms 200 \
  --resume
```

**Options:**
- `--input`: Input CSV file containing TinyURLs
- `--output`: Output CSV file (input + new Cardplay column)
- `--url-column`: Column name containing TinyURLs (default: "BBO")
- `--delay-ms`: Delay between URL resolutions in milliseconds (default: 200)
- `--batch-size`: Number of requests before a longer pause (default: 10)
- `--batch-delay-ms`: Pause duration after each batch (default: 2000)
- `--resume`: Skip rows that already have cardplay data

**Output format:**

The Cardplay column contains trick-by-trick notation:
```
D2-DA-D6-D5|S3-S2-SQ-SA|DK-D4-D3-D9|...
```
Each trick is separated by `|`, cards within a trick by `-`.

### Step 2: Analyze Double-Dummy Costs

```bash
bbo-csv analyze-dd \
  --input "Hand Records with Cardplay.csv" \
  --output "Hand Records with DD Analysis.csv" \
  --resume
```

**Options:**
- `--input`: CSV with cardplay data
- `--output`: Output CSV with DD analysis column added
- `--resume`: Skip rows that already have DD analysis

**Output format:**

The DD_Analysis column shows the cost (in tricks) of each card played:
```
T1:0,0,0,0|T2:0,0,1,0|T3:0,0,0,0|...
```
- `0` = optimal or equivalent play
- Positive number = tricks lost by suboptimal play

## CSV File Format

The tool expects a CSV with at minimum:
- A URL column (default name "BBO") containing TinyURLs to BBO hand records
- A "Ref #" column for identifying rows (used for resume functionality)

The tool preserves all existing columns and appends new ones.

## How It Works

1. **TinyURL Resolution**: Each TinyURL is resolved to its full BBO handviewer URL
2. **LIN Extraction**: The `lin` parameter is extracted from the URL
3. **LIN Parsing**: The LIN format is parsed to extract:
   - Player names
   - Deal (all four hands)
   - Auction with alerts/annotations
   - Complete cardplay sequence
   - Claim (if any)
4. **DD Analysis**: For each card played, the double-dummy solver computes:
   - Optimal result from current position
   - Actual result after the card was played
   - Cost = difference (0 = optimal)

## Rate Limiting

The tool includes configurable rate limiting to avoid being blocked by TinyURL:
- Default: 200ms delay between requests
- Batch mode: Pause after every N requests
- Exponential backoff on errors

## Resume Functionality

If processing is interrupted (crash, rate limit, etc.), the tool can resume:
- Reads the output file to find already-processed rows
- Only processes rows missing the target column
- Safe to run multiple times

## LIN Format Reference

BBO uses the LIN (Linear) format to encode hand data in URLs:

| Code | Meaning | Example |
|------|---------|---------|
| `pn` | Player names | `pn\|South,West,North,East\|` |
| `md` | Make deal | `md\|3S7643HAKQT43DA74C,...\|` |
| `sv` | Vulnerability | `sv\|o\|` (o=none, b=both, n=NS, e=EW) |
| `mb` | Make bid | `mb\|1C!\|` |
| `an` | Annotation | `an\|could be short\|` |
| `pc` | Play card | `pc\|D2\|` |
| `mc` | Make claim | `mc\|10\|` |

## Dependencies

- `bridge-solver`: Double-dummy solver library (local dependency)
- `reqwest`: HTTP client for URL resolution
- `url`: URL parsing
- `csv`: CSV reading/writing
- `clap`: CLI argument parsing
