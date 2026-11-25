//! Pure-std utilities: RFC 3339 UTC timestamp formatting, SHA-256 hashing, hex encoding,
//! and strict read-only SQL pre-validation.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{DbError, Result};

/// Return the current wall-clock time formatted as UTC RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn utc_now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_unix_secs_rfc3339(secs)
}

/// Convert Unix epoch seconds to UTC RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// Uses Howard Hinnant's civil-from-days algorithm so no date/time crate is required.
pub fn format_unix_secs_rfc3339(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Encode bytes as lowercase hexadecimal.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Compute the lowercase hex SHA-256 digest of a byte slice (FIPS 180-4).
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Compute the lowercase hex SHA-256 digest of a file on disk.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|err| DbError::io("读取文件", path, err))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|err| DbError::io("读取文件", path, err))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

/// Normalize a relative filesystem path to forward-slash UTF-8 form for cross-platform determinism.
pub(crate) fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Streaming pure-Rust SHA-256 hasher (FIPS 180-4).
pub(crate) struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    const INIT: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    pub(crate) fn new() -> Self {
        Self {
            state: Self::INIT,
            block: [0u8; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    pub(crate) fn update(&mut self, mut input: &[u8]) {
        self.total_len = self.total_len.wrapping_add(input.len() as u64);

        if self.block_len > 0 {
            let take = (64 - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }

        let (chunks, rem) = input.as_chunks::<64>();
        for chunk in chunks {
            self.compress(chunk);
        }
        input = rem;

        if !input.is_empty() {
            self.block[..input.len()].copy_from_slice(input);
            self.block_len = input.len();
        }
    }

    pub(crate) fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;

        if self.block_len > 56 {
            self.block[self.block_len..64].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block_len = 0;
        }

        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);

        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (slot, chunk) in w[..16].iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*chunk);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (&k, &wi) in Self::K.iter().zip(&w) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k)
                .wrapping_add(wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

/// Validate that `sql` is a single read-only query (`SELECT`, `WITH`, or `VALUES`) and return
/// the trimmed statement without a trailing semicolon.
///
/// Callers must additionally check `rusqlite::Statement::readonly()` after preparing on a
/// read-only connection (`query_only = ON`).
pub fn validate_readonly_sql(sql: &str) -> Result<&str> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(DbError::WriteQueryRejected {
            detail: "SQL 语句为空".to_owned(),
        });
    }

    let bytes = trimmed.as_bytes();
    let mut i = 0;
    let mut first_keyword: Option<String> = None;
    let mut saw_semicolon = false;
    let mut statement_end = bytes.len();

    while i < bytes.len() {
        let b = bytes[i];
        // Skip whitespace.
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Skip `-- ...` line comment.
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip `/* ... */` block comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(DbError::WriteQueryRejected {
                    detail: "SQL 注释未闭合".to_owned(),
                });
            }
            continue;
        }

        if saw_semicolon {
            return Err(DbError::WriteQueryRejected {
                detail: "只读查询仅允许单条语句，禁止用分号拼接多条 SQL".to_owned(),
            });
        }

        if b == b';' {
            saw_semicolon = true;
            statement_end = i;
            i += 1;
            continue;
        }

        // Skip quoted strings / identifiers so embedded semicolons are not treated as separators.
        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == quote {
                    if i + 1 < bytes.len() && bytes[i + 1] == quote {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(DbError::WriteQueryRejected {
                    detail: "SQL 字符串或标识符引号未闭合".to_owned(),
                });
            }
            continue;
        }

        if b == b'[' {
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == b']' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(DbError::WriteQueryRejected {
                    detail: "SQL 方括号标识符未闭合".to_owned(),
                });
            }
            continue;
        }

        if first_keyword.is_none() {
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                first_keyword = Some(trimmed[start..i].to_ascii_uppercase());
                continue;
            }
            return Err(DbError::WriteQueryRejected {
                detail: "只读查询必须以 SELECT / WITH / VALUES 开头".to_owned(),
            });
        }

        i += 1;
    }

    let Some(keyword) = first_keyword else {
        return Err(DbError::WriteQueryRejected {
            detail: "SQL 语句为空或仅含注释".to_owned(),
        });
    };

    if !matches!(keyword.as_str(), "SELECT" | "WITH" | "VALUES") {
        return Err(DbError::WriteQueryRejected {
            detail: format!("仅允许 SELECT / WITH / VALUES 只读查询，已拒绝 {keyword} 语句"),
        });
    }

    Ok(trimmed[..statement_end].trim())
}

#[cfg(test)]
mod tests {
    use super::{format_unix_secs_rfc3339, sha256_hex, validate_readonly_sql};

    #[test]
    fn sha256_matches_nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rfc3339_formatting_matches_known_timestamps() {
        assert_eq!(format_unix_secs_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            format_unix_secs_rfc3339(1_700_000_000),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            format_unix_secs_rfc3339(1_709_164_800),
            "2024-02-29T00:00:00Z"
        );
    }

    #[test]
    fn readonly_sql_validator_accepts_select_and_rejects_writes() {
        assert_eq!(
            validate_readonly_sql("  -- comment\nSELECT 1; ").unwrap(),
            "-- comment\nSELECT 1"
        );
        assert_eq!(
            validate_readonly_sql("SELECT ';' AS semi").unwrap(),
            "SELECT ';' AS semi"
        );
        assert!(validate_readonly_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
        assert!(validate_readonly_sql("INSERT INTO meta VALUES ('a', 'b')").is_err());
        assert!(validate_readonly_sql("DELETE FROM meta").is_err());
        assert!(validate_readonly_sql("UPDATE meta SET value = 'x'").is_err());
        assert!(validate_readonly_sql("DROP TABLE meta").is_err());
        assert!(validate_readonly_sql("PRAGMA query_only = OFF").is_err());
        assert!(validate_readonly_sql("SELECT 1; DROP TABLE meta").is_err());
    }
}
