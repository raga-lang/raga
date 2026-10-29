//! Format strings: the first argument of `String.format` and `$`.
//!
//! The format follows C's `printf`. Text is copied, and each `%` says where a value goes and how it is written:
//!
//! ```text
//! %[flags][width][.precision][length]conversion
//!   flags       - + space # 0
//!   width       digits, or * to take it from a value
//!   precision   . then digits, or .* to take it from a value
//!   length      h hh l ll j z t L   accepted and ignored, the compiler knows the type of each value
//!   conversion  d i u x X o   f F e E g G   c   s   p        and %% for a percent sign
//! ```
//!
//! A format that is a string literal is checked while the program is built: how many values it takes, and the kind of
//! value each place takes. This module reads the format and lists those places; checking the arguments against them is
//! done in [`call`](super::call).

/// What a place in the format takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// `%d` `%i`
    Signed,
    /// `%u`
    Unsigned,
    /// `%x` `%X` `%o`
    Integer,
    /// `%f` `%F` `%e` `%E` `%g` `%G`
    Float,
    /// `%c`
    Char,
    /// `%s`
    Str,
    /// `%p`
    Pointer,
    /// the `*` of a width or a precision: a whole number that sets the size of the next value
    Star,
}

/// One value the format reads, with the text of the `%...` it belongs to for messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Place {
    pub spec: String,
    pub kind: Kind,
}

/// The values a format reads, in order. A `*` reads its own value before the value it sizes, so `%*d` reads two.
pub(super) fn parse(format: &str) -> Result<Vec<Place>, String> {
    let chars: Vec<char> = format.chars().collect();
    let mut places = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        match chars.get(i) {
            None => {
                return Err(
                    "the format ends with a lone `%`: write `%%` for a percent sign".to_string(),
                );
            }
            Some('%') => {
                i += 1;
                continue;
            }
            Some(_) => {}
        }
        while chars.get(i).is_some_and(|c| "-+ #0".contains(*c)) {
            i += 1;
        }
        let mut stars = 0;
        if chars.get(i) == Some(&'*') {
            stars += 1;
            i += 1;
        } else {
            i = skip_digits(&chars, i);
        }
        if chars.get(i) == Some(&'.') {
            i += 1;
            if chars.get(i) == Some(&'*') {
                stars += 1;
                i += 1;
            } else {
                i = skip_digits(&chars, i);
            }
        }
        while chars.get(i).is_some_and(|c| "hljztL".contains(*c)) {
            i += 1;
        }
        let Some(&conversion) = chars.get(i) else {
            let spec: String = chars[start..].iter().collect();
            return Err(format!(
                "the format ends inside `{spec}`, it needs a letter such as d or s"
            ));
        };
        i += 1;
        let spec: String = chars[start..i].iter().collect();
        let kind = match conversion {
            'd' | 'i' => Kind::Signed,
            'u' => Kind::Unsigned,
            'x' | 'X' | 'o' => Kind::Integer,
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => Kind::Float,
            'c' => Kind::Char,
            's' => Kind::Str,
            'p' => Kind::Pointer,
            _ => return Err(format!("`{spec}` is not a format")),
        };
        for _ in 0..stars {
            places.push(Place {
                spec: spec.clone(),
                kind: Kind::Star,
            });
        }
        places.push(Place { spec, kind });
    }
    Ok(places)
}

fn skip_digits(chars: &[char], mut i: usize) -> usize {
    while chars.get(i).is_some_and(char::is_ascii_digit) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(format: &str) -> Vec<Kind> {
        parse(format).unwrap().iter().map(|p| p.kind).collect()
    }

    #[test]
    fn plain_text_takes_no_values() {
        assert_eq!(kinds(""), []);
        assert_eq!(kinds("hello world"), []);
        assert_eq!(kinds("100%% sure"), []);
    }

    #[test]
    fn every_conversion() {
        use Kind::*;
        assert_eq!(kinds("%d %i %u"), [Signed, Signed, Unsigned]);
        assert_eq!(kinds("%x %X %o"), [Integer, Integer, Integer]);
        assert_eq!(kinds("%f %F %e %E %g %G"), [Float; 6]);
        assert_eq!(kinds("%c %s %p"), [Char, Str, Pointer]);
    }

    #[test]
    fn flags_width_and_precision_do_not_change_what_a_place_takes() {
        use Kind::*;
        assert_eq!(
            kinds("%-10s %05d %8.3f %+d %#x"),
            [Str, Signed, Float, Signed, Integer]
        );
        assert_eq!(kinds("%.2f [%5d] [%-5d]"), [Float, Signed, Signed]);
        assert_eq!(kinds("% d"), [Signed]);
    }

    #[test]
    fn length_modifiers_are_accepted_and_ignored() {
        use Kind::*;
        assert_eq!(
            kinds("%ld %lld %hhd %zu %Lf"),
            [Signed, Signed, Signed, Unsigned, Float]
        );
    }

    #[test]
    fn a_star_reads_a_value_before_the_value_it_sizes() {
        use Kind::*;
        assert_eq!(kinds("%*d"), [Star, Signed]);
        assert_eq!(kinds("%.*f"), [Star, Float]);
        assert_eq!(kinds("%*.*f"), [Star, Star, Float]);
        assert_eq!(kinds("%*d|%.*f"), [Star, Signed, Star, Float]);
    }

    #[test]
    fn a_place_keeps_its_text_for_messages() {
        let places = parse("a %-5d b %s").unwrap();
        assert_eq!(places[0].spec, "%-5d");
        assert_eq!(places[1].spec, "%s");
    }

    #[test]
    fn unknown_conversions_are_reported() {
        assert_eq!(parse("%q").unwrap_err(), "`%q` is not a format");
        assert_eq!(parse("x %5n").unwrap_err(), "`%5n` is not a format");
        assert_eq!(parse("%y").unwrap_err(), "`%y` is not a format");
    }

    #[test]
    fn a_format_cannot_end_inside_a_place() {
        assert!(parse("100%").unwrap_err().contains("lone `%`"));
        assert!(parse("%5").unwrap_err().contains("ends inside `%5`"));
        assert!(parse("%-").unwrap_err().contains("ends inside"));
    }
}
