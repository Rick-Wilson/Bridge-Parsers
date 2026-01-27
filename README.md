# Bridge-Parsers

A Rust library and CLI for reading and writing bridge file formats.

## Features

- **PBN (Portable Bridge Notation)**: Read and write `.pbn` files with full support for deals, contracts, and metadata
- **BWS (BridgeWebs)**: Extract data from Bridgemate `.bws` database files (requires mdbtools)
- **LIN (BBO Linear)**: Parse BBO hand records including auction, cardplay, and claims
- **XLSX Export**: Generate Excel spreadsheets with hand records and game results
- **ACBL Integration**: Fetch masterpoint data from ACBL Live for Clubs
- **URL Resolution**: Resolve TinyURL/bit.ly links to BBO hand records with rate limiting

## Installation

```bash
cargo install --git https://github.com/Rick-Wilson/Bridge-Parsers
```

Or build from source:

```bash
git clone https://github.com/Rick-Wilson/Bridge-Parsers
cd Bridge-Parsers
cargo build --release
```

### BWS Support

Reading `.bws` files requires [mdbtools](https://github.com/mdbtools/mdbtools):

```bash
# macOS
brew install mdbtools

# Ubuntu/Debian
sudo apt-get install mdbtools
```

## CLI Usage

### Convert Files

```bash
# PBN to Excel
bridge-parsers convert input.pbn -o output.xlsx

# BWS to Excel (with game results)
bridge-parsers convert game.bws -o results.xlsx

# BWS to PBN (hand records only)
bridge-parsers convert game.bws -o hands.pbn
```

### Combine PBN and BWS

Merge hand records from a PBN file with game results from a BWS file:

```bash
bridge-parsers combine hands.pbn scores.bws -o combined.xlsx
```

### With ACBL Masterpoints

Add player masterpoint data from ACBL Live for Clubs:

```bash
bridge-parsers convert game.bws -o results.xlsx \
  --masterpoints "https://live.acbl.org/club-results/..."
```

### File Information

```bash
bridge-parsers info game.bws
bridge-parsers info hands.pbn
```

### Validation

```bash
bridge-parsers validate hands.pbn
bridge-parsers validate game.bws
```

## Library Usage

```rust
use bridge_parsers::pbn::{read_pbn_file, write_pbn_file};
use bridge_parsers::bws::read_bws;
use bridge_parsers::lin::parse_lin_from_url;
use bridge_parsers::xlsx::write_boards_to_xlsx;

// Read PBN file
let boards = read_pbn_file("hands.pbn")?;

// Read BWS database
let bws_data = read_bws(&PathBuf::from("game.bws"))?;

// Parse LIN from BBO URL
let lin_data = parse_lin_from_url("https://www.bridgebase.com/tools/handviewer.html?lin=...")?;

// Write to Excel
write_boards_to_xlsx(&boards, &PathBuf::from("output.xlsx"))?;
```

## Related Projects

- **[EDGAR-Defense-Toolkit](https://github.com/Rick-Wilson/EDGAR-Defense-Toolkit)**: Double-dummy analysis tools for detecting suspicious play patterns
- **[bridge-types](https://github.com/Rick-Wilson/bridge-types)**: Core bridge data types (Board, Deal, Card, etc.)
- **[bridge-solver](https://github.com/Rick-Wilson/bridge-solver)**: Double-dummy solver engine

## File Format References

### PBN (Portable Bridge Notation)
Standard format for bridge hand records. See [PBN Standard](https://www.tistis.nl/pbn/).

### BWS (BridgeWebs)
Microsoft Access database format used by Bridgemate scoring devices.

### LIN (BBO Linear)
Pipe-delimited format used by Bridge Base Online for hand records in URLs.

| Code | Meaning | Example |
|------|---------|---------|
| `pn` | Player names | `pn\|South,West,North,East\|` |
| `md` | Make deal | `md\|3S7643HAKQT43DA74C,...\|` |
| `sv` | Vulnerability | `sv\|o\|` (o=none, b=both, n=NS, e=EW) |
| `mb` | Make bid | `mb\|1C!\|` |
| `an` | Annotation | `an\|could be short\|` |
| `pc` | Play card | `pc\|D2\|` |
| `mc` | Make claim | `mc\|10\|` |

## License

Unlicense
