//! 密码工具：密码生成器与强度评估（设计文档 3.4）。

use rand::{rngs::OsRng, seq::SliceRandom, Rng};

const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SPECIAL: &[u8] = b"!@#$%^&*()-_=+[]{};:,.<>?/";

/// 生成随机密码。
///
/// * `length`: 6..=32；
/// * `upper/lower/digit/special`: 是否包含对应字符集；
/// * 至少一个字符集被勾选。
pub fn generate(
    length: usize,
    upper: bool,
    lower: bool,
    digit: bool,
    special: bool,
) -> Result<String, String> {
    if length < 6 || length > 32 {
        return Err("密码长度需在 6~32 之间".into());
    }
    let mut pool: Vec<u8> = Vec::new();
    if upper {
        pool.extend_from_slice(UPPERCASE);
    }
    if lower {
        pool.extend_from_slice(LOWERCASE);
    }
    if digit {
        pool.extend_from_slice(DIGITS);
    }
    if special {
        pool.extend_from_slice(SPECIAL);
    }
    if pool.is_empty() {
        // 兜底：至少保留小写，避免空池（上层 UI 正常不会触发）。
        pool.extend_from_slice(LOWERCASE);
    }

    let selected_sets: Vec<&[u8]> = [
        (upper, UPPERCASE.as_ref()),
        (lower, LOWERCASE.as_ref()),
        (digit, DIGITS.as_ref()),
        (special, SPECIAL.as_ref()),
    ]
    .into_iter()
    .filter_map(|(selected, chars)| selected.then_some(chars))
    .collect();

    let mut result = Vec::with_capacity(length);
    // 先保证每个勾选的字符集至少出现一次，避免“已勾选但生成结果完全没有该类字符”的误导。
    if length >= selected_sets.len() {
        for chars in &selected_sets {
            result.push(chars[OsRng.gen_range(0..chars.len())]);
        }
    }
    while result.len() < length {
        result.push(pool[OsRng.gen_range(0..pool.len())]);
    }
    result.shuffle(&mut OsRng);
    String::from_utf8(result).map_err(|e| format!("生成密码失败: {e}"))
}

/// 密码强度评估（0-4 分）。返回 (等级 0~4, 标签, 说明)。
/// 等级：0=极弱 1=弱 2=中 3=强 4=极强。
pub fn strength(password: &str) -> (u8, &'static str, String) {
    let len = password.chars().count();
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password
        .chars()
        .any(|c| !c.is_alphanumeric() && !c.is_whitespace());

    let charsets = [has_lower, has_upper, has_digit, has_special]
        .iter()
        .filter(|b| **b)
        .count();

    // 简单信息熵估算：pool_size^len 的对数（log2）。
    let pool: f64 = if len == 0 {
        1.0
    } else {
        let mut p = 0.0;
        if has_upper {
            p += UPPERCASE.len() as f64;
        }
        if has_lower {
            p += LOWERCASE.len() as f64;
        }
        if has_digit {
            p += DIGITS.len() as f64;
        }
        if has_special {
            p += SPECIAL.len() as f64;
        }
        p
    };
    let bits = if pool <= 1.0 {
        0.0
    } else {
        (len as f64) * pool.log2()
    };

    let (score, label) = if len < 6 || bits < 28.0 {
        (0u8, "极弱")
    } else if len < 8 || bits < 40.0 {
        (1, "弱")
    } else if len < 12 || bits < 60.0 {
        (2, "中")
    } else if len < 16 || bits < 88.0 {
        (3, "强")
    } else {
        (4, "极强")
    };

    let detail = format!("长度 {len}，字符集 {charsets}/4，估算熵约 {:.0} bit", bits);
    (score, label, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_within_bounds() {
        for len in [6usize, 10, 16, 20, 32] {
            let p = generate(len, true, true, true, true).unwrap();
            assert_eq!(p.chars().count(), len);
        }
    }

    #[test]
    fn invalid_length_rejected() {
        assert!(generate(5, true, true, true, true).is_err());
        assert!(generate(33, true, true, true, true).is_err());
    }

    #[test]
    fn only_digits_generated() {
        let p = generate(12, false, false, true, false).unwrap();
        assert!(p.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn selected_charsets_are_present() {
        let p = generate(16, true, true, true, true).unwrap();
        assert!(p.chars().any(|c| c.is_ascii_uppercase()));
        assert!(p.chars().any(|c| c.is_ascii_lowercase()));
        assert!(p.chars().any(|c| c.is_ascii_digit()));
        assert!(p.chars().any(|c| !c.is_alphanumeric()));
    }

    #[test]
    fn charset_inclusion() {
        // 校验：勾选的字符集，其字符确实来自相应方言（不做“每类必出现”的强断言，
        // 因为随机生成不保证每类都覆盖）
        let allowed: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_=+[]{};:,.<>?/"
            .chars()
            .collect();
        for _ in 0..50 {
            let p = generate(24, true, true, true, true).unwrap();
            assert!(
                p.chars().all(|c| allowed.contains(&c)),
                "生成了超集外字符: {p}"
            );
        }
    }

    #[test]
    fn randomness_variation() {
        let a = generate(16, true, true, true, true).unwrap();
        let b = generate(16, true, true, true, true).unwrap();
        assert_ne!(a, b, "连续生成不应相同");
    }

    #[test]
    fn strength_levels() {
        assert_eq!(strength("a").0, 0);
        assert_eq!(strength("123456").0, 0); // 太短
        assert!(strength("P@ssw0rdA!B2C3D4E5F6G7H8").0 >= 4);
        // 期望：长且多字符集的密码强度更高
        let short = strength("abc123").0;
        let long = strength("Ab1!Xy9#Mn3@Qw0$Zk7%").0;
        assert!(long >= short);
    }
}
