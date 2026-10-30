//! ECMA-402's `Intl` through script (gap batch D, D63): what the
//! bindings answer, as the reference browser answers it.

use crate::Completion;
use crate::tests::{Recorder, realm, script};

/// The string each script ends with, each run in a realm of its own.
fn evaluated(source: &str) -> String {
    let mut realm = realm();
    let mut host = Recorder::default();
    match realm.run(&script(source), &mut host) {
        Ok(Completion::String(text)) => text,
        other => panic!("{source}: {other:?}"),
    }
}

fn check(cases: &[(&str, &str)]) {
    // Every mismatch, not the first alone.
    let mut bad = Vec::new();
    for (source, expected) in cases {
        let got = evaluated(source);
        if got != *expected {
            bad.push(format!(
                "{source}
  want {expected:?}
  got  {got:?}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{}",
        bad.join(
            "
"
        )
    );
}

#[test]
fn numbers_format_as_the_locale_writes_them() {
    check(&[
        (
            "new Intl.NumberFormat('en-US').format(1234567.891)",
            "1,234,567.891",
        ),
        (
            "new Intl.NumberFormat('de-DE').format(1234567.891)",
            "1.234.567,891",
        ),
        (
            "new Intl.NumberFormat('fr').format(1234567.891)",
            "1\u{202f}234\u{202f}567,891",
        ),
        (
            "new Intl.NumberFormat('hi-IN').format(12345678)",
            "1,23,45,678",
        ),
        (
            "new Intl.NumberFormat('ar-EG').format(1234.5)",
            "\u{661}\u{66c}\u{662}\u{663}\u{664}\u{66b}\u{665}",
        ),
        ("(0.256).toLocaleString('en', {style:'percent'})", "26%"),
        (
            "(0.256).toLocaleString('de', {style:'percent'})",
            "26\u{a0}%",
        ),
        ("(1234.5).toLocaleString()", "1,234.5"),
        ("(1234.5).toLocaleString('de')", "1.234,5"),
        (
            "new Intl.NumberFormat('en').format(12345678901234567890n)",
            "12,345,678,901,234,567,890",
        ),
        ("(12345n).toLocaleString('en')", "12,345"),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 0, roundingMode: 'halfEven'}).format(2.5)",
            "2",
        ),
        ("new Intl.NumberFormat('es').format(1234)", "1234"),
        (
            "new Intl.NumberFormat('es', {useGrouping: 'always'}).format(1234)",
            "1.234",
        ),
        (
            "new Intl.NumberFormat('es', {useGrouping: 'always'}).format(-1234.5)",
            "-1.234,5",
        ),
        (
            "new Intl.NumberFormat('hi-IN', {useGrouping: 'always'}).format(1234567)",
            "12,34,567",
        ),
        (
            "new Intl.NumberFormat('en', {useGrouping: 'always'}).format(123)",
            "123",
        ),
        (
            "new Intl.NumberFormat('en').format('1234.5678901234567890123')",
            "1,234.568",
        ),
        ("new Intl.NumberFormat('en').format(-0)", "-0"),
        ("new Intl.NumberFormat('en').format(NaN)", "NaN"),
        ("new Intl.NumberFormat('en').format(-Infinity)", "-\u{221e}"),
        ("Intl.NumberFormat('en').format(2)", "2"),
    ]);
}

#[test]
fn digit_options_round_and_pad() {
    check(&[
        (
            "new Intl.NumberFormat('en', {minimumFractionDigits: 2}).format(3)",
            "3.00",
        ),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 0}).format(2.5)",
            "3",
        ),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 0}).format(-2.5)",
            "-3",
        ),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 1}).format(0.96)",
            "1",
        ),
        (
            "new Intl.NumberFormat('en', {maximumSignificantDigits: 3}).format(123456)",
            "123,000",
        ),
        (
            "new Intl.NumberFormat('en', {minimumSignificantDigits: 5}).format(1.5)",
            "1.5000",
        ),
        (
            "new Intl.NumberFormat('en', {maximumSignificantDigits: 2}).format(0.0012345)",
            "0.0012",
        ),
        (
            "new Intl.NumberFormat('en', {minimumIntegerDigits: 3}).format(7)",
            "007",
        ),
        (
            "new Intl.NumberFormat('en', {useGrouping: false}).format(1234567)",
            "1234567",
        ),
        (
            "new Intl.NumberFormat('en', {signDisplay: 'always'}).format(5)",
            "+5",
        ),
        (
            "new Intl.NumberFormat('en', {signDisplay: 'exceptZero'}).format(0)",
            "0",
        ),
        (
            "new Intl.NumberFormat('en', {signDisplay: 'never'}).format(-5)",
            "5",
        ),
        (
            "new Intl.NumberFormat('en', {roundingMode: 'floor', maximumFractionDigits: 0}).format(2.9)",
            "2",
        ),
        (
            "new Intl.NumberFormat('en', {roundingIncrement: 5, maximumFractionDigits: 2, minimumFractionDigits: 2}).format(1.234)",
            "1.25",
        ),
        (
            "new Intl.NumberFormat('en', {minimumFractionDigits: 2, trailingZeroDisplay: 'stripIfInteger'}).format(4)",
            "4",
        ),
        (
            "new Intl.NumberFormat('en', {notation: 'scientific'}).format(123456)",
            "1.235E5",
        ),
        (
            "new Intl.NumberFormat('en', {notation: 'scientific'}).format(0.00012)",
            "1.2E-4",
        ),
        (
            "new Intl.NumberFormat('en', {notation: 'engineering'}).format(123456)",
            "123.456E3",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'percent', maximumFractionDigits: 1}).format(0.12345)",
            "12.3%",
        ),
    ]);
}

#[test]
fn number_parts_and_resolved_options_are_ecma_402s() {
    check(&[
        (
            "JSON.stringify(new Intl.NumberFormat('en').formatToParts(-1234.5))",
            r#"[{"type":"minusSign","value":"-"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"5"}]"#,
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('en', {style: 'percent'}).formatToParts(0.5))",
            r#"[{"type":"integer","value":"50"},{"type":"percentSign","value":"%"}]"#,
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('en').resolvedOptions())",
            r#"{"locale":"en","numberingSystem":"latn","style":"decimal","minimumIntegerDigits":1,"minimumFractionDigits":0,"maximumFractionDigits":3,"useGrouping":"auto","notation":"standard","signDisplay":"auto","roundingIncrement":1,"roundingMode":"halfExpand","roundingPriority":"auto","trailingZeroDisplay":"auto"}"#,
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('de', {maximumSignificantDigits: 3}).resolvedOptions())",
            r#"{"locale":"de","numberingSystem":"latn","style":"decimal","minimumIntegerDigits":1,"minimumSignificantDigits":1,"maximumSignificantDigits":3,"useGrouping":"auto","notation":"standard","signDisplay":"auto","roundingIncrement":1,"roundingMode":"halfExpand","roundingPriority":"auto","trailingZeroDisplay":"auto"}"#,
        ),
        (
            "new Intl.NumberFormat('ar-EG').resolvedOptions().numberingSystem",
            "arab",
        ),
        (
            "new Intl.NumberFormat('en-u-nu-arab').resolvedOptions().locale",
            "en-u-nu-arab",
        ),
        (
            "new Intl.NumberFormat('en-u-nu-arab').format(12)",
            "\u{661}\u{662}",
        ),
        // The bound format is one function for each formatter.
        (
            "var f = new Intl.NumberFormat('en'); String(f.format === f.format && f.format.name === '')",
            "true",
        ),
        (
            "[1, 2, 3].map(new Intl.NumberFormat('en', {minimumFractionDigits: 1}).format).join(' ')",
            "1.0 2.0 3.0",
        ),
    ]);
}

#[test]
fn plural_rules_select_cardinal_and_ordinal_categories() {
    check(&[
        (
            "new Intl.PluralRules('en', {type: 'ordinal'}).select(2)",
            "two",
        ),
        (
            "new Intl.PluralRules('en', {type: 'ordinal'}).select(3)",
            "few",
        ),
        (
            "new Intl.PluralRules('en', {type: 'ordinal'}).select(11)",
            "other",
        ),
        (
            "new Intl.PluralRules('en', {type: 'ordinal'}).select(21)",
            "one",
        ),
        ("new Intl.PluralRules('en').select(1)", "one"),
        ("new Intl.PluralRules('en').select(0)", "other"),
        (
            "new Intl.PluralRules('en', {minimumFractionDigits: 1}).select(1)",
            "other",
        ),
        ("new Intl.PluralRules('ar').select(3)", "few"),
        ("new Intl.PluralRules('ru').select(5)", "many"),
        ("new Intl.PluralRules('fr').select(1.5)", "one"),
        (
            "JSON.stringify(new Intl.PluralRules('en').resolvedOptions())",
            r#"{"locale":"en","type":"cardinal","minimumIntegerDigits":1,"minimumFractionDigits":0,"maximumFractionDigits":3,"pluralCategories":["one","other"],"roundingIncrement":1,"roundingMode":"halfExpand","roundingPriority":"auto","trailingZeroDisplay":"auto"}"#,
        ),
        (
            "JSON.stringify(new Intl.PluralRules('en', {type: 'ordinal'}).resolvedOptions().pluralCategories)",
            r#"["one","two","few","other"]"#,
        ),
        (
            "try { Intl.PluralRules('en'); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
    ]);
}

#[test]
fn collators_compare_by_the_locales_order() {
    check(&[
        (
            "['b', 'a', 'A'].sort(new Intl.Collator('en').compare).join()",
            "a,A,b",
        ),
        (
            "['b', 'a', 'B', 'A'].sort(new Intl.Collator('en', {caseFirst: 'upper'}).compare).join()",
            "A,a,B,b",
        ),
        (
            "['a10', 'a2', 'a1'].sort(new Intl.Collator('en', {numeric: true}).compare).join()",
            "a1,a2,a10",
        ),
        (
            "['a10', 'a2', 'a1'].sort(new Intl.Collator('en').compare).join()",
            "a1,a10,a2",
        ),
        (
            "String('\u{e4}'.localeCompare('a', 'de', {sensitivity: 'base'}))",
            "0",
        ),
        (
            "String('\u{e4}'.localeCompare('a', 'de', {sensitivity: 'accent'}))",
            "1",
        ),
        (
            "String('a'.localeCompare('A', 'en', {sensitivity: 'accent'}))",
            "0",
        ),
        (
            "String('a'.localeCompare('A', 'en', {sensitivity: 'case'}))",
            "-1",
        ),
        ("String('a'.localeCompare('b'))", "-1"),
        ("String('b'.localeCompare('a'))", "1"),
        ("String('a'.localeCompare('a'))", "0"),
        (
            "['\u{e9}', 'e', 'f'].sort((x, y) => x.localeCompare(y)).join()",
            "e,\u{e9},f",
        ),
        (
            "JSON.stringify(new Intl.Collator('en-u-kn').resolvedOptions())",
            r#"{"locale":"en-u-kn","usage":"sort","sensitivity":"variant","ignorePunctuation":false,"collation":"default","numeric":true,"caseFirst":"false"}"#,
        ),
        (
            "var c = new Intl.Collator('en'); String(c.compare === c.compare && c.compare.length)",
            "2",
        ),
        ("Intl.Collator('en').resolvedOptions().usage", "sort"),
    ]);
}

#[test]
fn dates_format_in_utc_as_the_reference_does() {
    let at = "Date.UTC(2020, 0, 2, 15, 4, 5)";
    let format = |options: &str| {
        format!("new Intl.DateTimeFormat('en-US', {{timeZone: 'UTC', {options}}}).format({at})")
    };
    check(&[
        (
            "new Intl.DateTimeFormat('en-US', {timeZone: 'UTC'}).format(new Date(Date.UTC(2020, 0, 2)))",
            "1/2/2020",
        ),
        (
            &format("dateStyle: 'full', timeStyle: 'long'"),
            "Thursday, January 2, 2020 at 3:04:05 PM UTC",
        ),
        (
            &format("dateStyle: 'medium', timeStyle: 'short'"),
            "Jan 2, 2020, 3:04 PM",
        ),
        (&format("dateStyle: 'long'"), "January 2, 2020"),
        (&format("dateStyle: 'short'"), "1/2/20"),
        (&format("timeStyle: 'medium'"), "3:04:05 PM"),
        (
            &format("timeStyle: 'full'"),
            "3:04:05 PM Coordinated Universal Time",
        ),
        (
            &format("weekday: 'long', year: 'numeric', month: 'long', day: 'numeric'"),
            "Thursday, January 2, 2020",
        ),
        (
            &format("year: 'numeric', month: '2-digit', day: '2-digit'"),
            "01/02/2020",
        ),
        (&format("hour: '2-digit', minute: '2-digit'"), "03:04 PM"),
        (
            &format("hour: 'numeric', minute: '2-digit', hour12: false"),
            "15:04",
        ),
        (
            &format("hour: 'numeric', minute: '2-digit', hourCycle: 'h23'"),
            "15:04",
        ),
        (&format("month: 'short', day: 'numeric'"), "Jan 2"),
        (&format("month: 'long'"), "January"),
        (&format("year: 'numeric', month: 'long'"), "January 2020"),
        (&format("weekday: 'short'"), "Thu"),
        (&format("hour: 'numeric'"), "3 PM"),
        (
            &format!("new Date({at}).toLocaleString('en-US', {{timeZone: 'UTC'}})"),
            "1/2/2020, 3:04:05 PM",
        ),
        (
            &format!("new Date({at}).toLocaleTimeString('en-US', {{timeZone: 'UTC'}})"),
            "3:04:05 PM",
        ),
        (
            &format!("new Date({at}).toLocaleDateString('en-GB', {{timeZone: 'UTC'}})"),
            "02/01/2020",
        ),
        (
            &format!(
                "new Intl.DateTimeFormat('ja', {{timeZone: 'UTC', dateStyle: 'long'}}).format({at})"
            ),
            "2020\u{5e74}1\u{6708}2\u{65e5}",
        ),
        (
            &format!(
                "new Intl.DateTimeFormat('en-US', {{timeZone: '+05:30', timeStyle: 'short'}}).format({at})"
            ),
            "8:34 PM",
        ),
        ("new Date(NaN).toLocaleString()", "Invalid Date"),
    ]);
}

#[test]
fn dates_format_in_the_systems_zone_as_date_does() {
    // Whatever the system's zone, the formatter's local time is the
    // one `Date` makes from local fields.
    check(&[
        (
            "new Date(2020, 0, 2, 15, 4, 5).toLocaleString('en-US')",
            "1/2/2020, 3:04:05 PM",
        ),
        (
            "new Date(2020, 6, 2, 9, 4, 5).toLocaleString('en-US')",
            "7/2/2020, 9:04:05 AM",
        ),
        (
            "new Intl.DateTimeFormat('en-US').format(new Date(2020, 0, 2))",
            "1/2/2020",
        ),
        (
            "var z = new Intl.DateTimeFormat().resolvedOptions().timeZone; String(z.length > 0)",
            "true",
        ),
    ]);
}

#[test]
fn date_parts_and_resolved_options_are_ecma_402s() {
    check(&[
        (
            "JSON.stringify(new Intl.DateTimeFormat('en-US', {timeZone: 'UTC', hour: 'numeric', minute: 'numeric'}).formatToParts(Date.UTC(2020, 0, 2, 15, 4)))",
            "[{\"type\":\"hour\",\"value\":\"3\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"minute\",\"value\":\"04\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"dayPeriod\",\"value\":\"PM\"}]",
        ),
        (
            "new Intl.DateTimeFormat('en-US', {timeZone: 'UTC'}).formatToParts(0).map(p => p.type).join()",
            "month,literal,day,literal,year",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('en-US', {timeZone: 'UTC'}).resolvedOptions())",
            r#"{"locale":"en-US","calendar":"gregory","numberingSystem":"latn","timeZone":"UTC","year":"numeric","month":"numeric","day":"numeric"}"#,
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('en-US', {timeZone: 'Etc/GMT', hour: 'numeric'}).resolvedOptions())",
            r#"{"locale":"en-US","calendar":"gregory","numberingSystem":"latn","timeZone":"UTC","hourCycle":"h12","hour12":true,"hour":"numeric"}"#,
        ),
        (
            "new Intl.DateTimeFormat('en-GB', {timeZone: 'UTC', timeStyle: 'short'}).resolvedOptions().hourCycle",
            "h23",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('en', {timeZone: 'UTC', dateStyle: 'short'}).resolvedOptions())",
            r#"{"locale":"en","calendar":"gregory","numberingSystem":"latn","timeZone":"UTC","dateStyle":"short"}"#,
        ),
        (
            "new Intl.DateTimeFormat('th', {timeZone: 'UTC'}).resolvedOptions().calendar",
            "buddhist",
        ),
        (
            "try { new Intl.DateTimeFormat('en', {dateStyle: 'short', year: 'numeric'}); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        (
            "try { new Intl.DateTimeFormat('en', {timeZone: 'Mars/Olympus'}); 'no' } catch (e) { e.message }",
            "Invalid time zone specified: Mars/Olympus",
        ),
        (
            "try { new Intl.DateTimeFormat('en').format(8.64e15 + 1); 'no' } catch (e) { e.message }",
            "Invalid time value",
        ),
    ]);
}

#[test]
fn locales_canonicalize_and_negotiate() {
    check(&[
        (
            "JSON.stringify(Intl.getCanonicalLocales('EN-us'))",
            r#"["en-US"]"#,
        ),
        (
            "JSON.stringify(Intl.getCanonicalLocales(['en', 'EN', 'iw']))",
            r#"["en","he"]"#,
        ),
        (
            "JSON.stringify(Intl.getCanonicalLocales('en-u-kn-true'))",
            r#"["en-u-kn"]"#,
        ),
        ("JSON.stringify(Intl.getCanonicalLocales())", "[]"),
        (
            "try { Intl.getCanonicalLocales('en_US') } catch (e) { e.name + ': ' + e.message }",
            "RangeError: Invalid language tag: en_US",
        ),
        (
            "try { Intl.getCanonicalLocales([5]) } catch (e) { e.name }",
            "TypeError",
        ),
        ("new Intl.Locale('en-Latn-US').language", "en"),
        (
            "new Intl.Locale('en-Latn-US').script + new Intl.Locale('en-Latn-US').region",
            "LatnUS",
        ),
        (
            "new Intl.Locale('en-Latn-US-u-ca-buddhist').baseName",
            "en-Latn-US",
        ),
        (
            "new Intl.Locale('en-Latn-US-u-ca-buddhist').calendar",
            "buddhist",
        ),
        (
            "new Intl.Locale('en', {region: 'GB', calendar: 'gregory'}).toString()",
            "en-GB-u-ca-gregory",
        ),
        (
            "new Intl.Locale('en', {numeric: true, hourCycle: 'h23'}).toString()",
            "en-u-hc-h23-kn",
        ),
        ("String(new Intl.Locale('en-u-kn').numeric)", "true"),
        ("String(new Intl.Locale('en').region)", "undefined"),
        ("new Intl.Locale('en').maximize().toString()", "en-Latn-US"),
        (
            "new Intl.Locale('zh-TW').maximize().toString()",
            "zh-Hant-TW",
        ),
        ("new Intl.Locale('en-Latn-US').minimize().toString()", "en"),
        ("new Intl.Locale('iw').toString()", "he"),
        (
            "Intl.getCanonicalLocales(new Intl.Locale('de-DE'))[0]",
            "de-DE",
        ),
        (
            "try { Intl.Locale('en') } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "try { new Intl.Locale('en', {region: 'Q'}) } catch (e) { e.name }",
            "RangeError",
        ),
        (
            "JSON.stringify(Intl.NumberFormat.supportedLocalesOf(['xx', 'en-GB', 'de']))",
            r#"["en-GB","de"]"#,
        ),
        (
            "new Intl.NumberFormat('xx').resolvedOptions().locale",
            "en-US",
        ),
        (
            "new Intl.NumberFormat(['xx', 'fr-CA']).resolvedOptions().locale",
            "fr-CA",
        ),
        (
            "new Intl.Collator('de-u-co-phonebk').resolvedOptions().collation",
            "phonebk",
        ),
        (
            "new Intl.Collator('de-u-co-bogus').resolvedOptions().locale",
            "de",
        ),
        (
            "JSON.stringify(Intl.supportedValuesOf('calendar').slice(0, 3))",
            r#"["buddhist","chinese","coptic"]"#,
        ),
        (
            "String(Intl.supportedValuesOf('unit').includes('kilometer'))",
            "true",
        ),
        (
            "try { Intl.supportedValuesOf('colour') } catch (e) { e.name }",
            "RangeError",
        ),
    ]);
}

#[test]
fn intl_is_a_namespace_of_built_ins() {
    check(&[
        (
            "Object.prototype.toString.call(Intl) + Object.prototype.toString.call(new Intl.NumberFormat())",
            "[object Intl][object Intl.NumberFormat]",
        ),
        ("typeof Intl", "object"),
        (
            "String(Object.getOwnPropertyDescriptor(globalThis, 'Intl').enumerable)",
            "false",
        ),
        ("String(Object.keys(Intl).length)", "0"),
        (
            "Object.getOwnPropertyNames(Intl).sort().join()",
            "Collator,DateTimeFormat,DisplayNames,DurationFormat,Locale,NumberFormat,PluralRules,\
             Segmenter,getCanonicalLocales,supportedValuesOf",
        ),
        (
            "String(Object.keys(Intl.NumberFormat.prototype).length)",
            "0",
        ),
        (
            "Intl.DateTimeFormat.name + Intl.Locale.length + Intl.NumberFormat.length",
            "DateTimeFormat10",
        ),
        (
            "String(Object.getOwnPropertyDescriptor(Intl.NumberFormat, 'prototype').writable)",
            "false",
        ),
        (
            "typeof Object.getOwnPropertyDescriptor(Intl.Collator.prototype, 'compare').get",
            "function",
        ),
        (
            "Number.prototype.toLocaleString.length + '' + String.prototype.localeCompare.length",
            "01",
        ),
        (
            "try { Intl.NumberFormat.prototype.format } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "try { Intl.NumberFormat.prototype.resolvedOptions.call({}) } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "class Mine extends Intl.NumberFormat {} ; new Mine('de').format(1.5)",
            "1,5",
        ),
        // A page that replaces the built-ins changes nothing the bindings do.
        (
            "Array.prototype.push = null; Array.prototype.indexOf = null; String.prototype.toUpperCase = null; new Intl.NumberFormat('en', {style: 'currency', currency: 'eur'}).resolvedOptions().currency",
            "EUR",
        ),
    ]);
}

#[test]
fn hostile_options_are_bounded() {
    check(&[
        (
            "try { new Intl.NumberFormat('en', {maximumFractionDigits: 101}) } catch (e) { e.name + ': ' + e.message }",
            "RangeError: maximumFractionDigits value is out of range.",
        ),
        (
            "try { new Intl.NumberFormat('en', {maximumSignificantDigits: 22}) } catch (e) { e.name }",
            "RangeError",
        ),
        (
            "try { new Intl.NumberFormat('en', {minimumFractionDigits: 3, maximumFractionDigits: 2}) } catch (e) { e.name }",
            "RangeError",
        ),
        (
            "try { new Intl.NumberFormat('en', {style: 'currency'}) } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "try { new Intl.NumberFormat('en', {style: 'bogus'}) } catch (e) { e.message }",
            "Value bogus out of range for Intl.NumberFormat options property style",
        ),
        (
            "try { new Intl.NumberFormat({length: 1e9}) } catch (e) { e.name }",
            "RangeError",
        ),
        (
            "try { new Intl.NumberFormat('a'.repeat(300)) } catch (e) { e.name }",
            "RangeError",
        ),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 20}).format(0.1)",
            "0.1",
        ),
        (
            "new Intl.NumberFormat('en', {minimumFractionDigits: 100}).format(5e-324).length + ''",
            "102",
        ),
        (
            "new Intl.NumberFormat('en').format(1.7976931348623157e308).length + ''",
            "411",
        ),
        (
            "String(new Intl.NumberFormat('en').format('9'.repeat(5000)).length > 0)",
            "true",
        ),
    ]);
}

#[test]
fn currencies_are_written_by_symbol_code_or_name() {
    let usd = |options: &str, x: &str| {
        format!(
            "new Intl.NumberFormat('en', {{style: 'currency', currency: 'USD'{options}}}).format({x})"
        )
    };
    check(&[
        (&usd("", "1234.5"), "$1,234.50"),
        (&usd("", "-1234.5"), "-$1,234.50"),
        (&usd(", signDisplay: 'always'", "5"), "+$5.00"),
        (&usd(", currencySign: 'accounting'", "-5"), "($5.00)"),
        (&usd(", currencyDisplay: 'code'", "5"), "USD\u{a0}5.00"),
        (&usd(", currencyDisplay: 'name'", "5"), "5.00 US dollars"),
        (&usd(", maximumFractionDigits: 0", "5"), "$5"),
        (&usd("", "NaN"), "$NaN"),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'EUR'}).format(5)",
            "\u{20ac}5.00",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'CAD'}).format(5)",
            "CA$5.00",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'CAD', currencyDisplay: 'narrowSymbol'}).format(5)",
            "$5.00",
        ),
        (
            "new Intl.NumberFormat('de', {style: 'currency', currency: 'EUR'}).format(1234.5)",
            "1.234,50\u{a0}\u{20ac}",
        ),
        (
            "new Intl.NumberFormat('ja', {style: 'currency', currency: 'JPY'}).format(1234.5)",
            "\u{ffe5}1,235",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('en', {style: 'currency', currency: 'USD'}).formatToParts(-1234.5))",
            r#"[{"type":"minusSign","value":"-"},{"type":"currency","value":"$"},{"type":"integer","value":"1"},{"type":"group","value":","},{"type":"integer","value":"234"},{"type":"decimal","value":"."},{"type":"fraction","value":"50"}]"#,
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('de', {style: 'currency', currency: 'EUR'}).formatToParts(5))",
            "[{\"type\":\"integer\",\"value\":\"5\"},{\"type\":\"decimal\",\"value\":\",\"},{\"type\":\"fraction\",\"value\":\"00\"},{\"type\":\"literal\",\"value\":\"\u{a0}\"},{\"type\":\"currency\",\"value\":\"\u{20ac}\"}]",
        ),
    ]);
}

#[test]
fn units_are_written_by_their_names() {
    let unit = |locale: &str, options: &str, x: &str| {
        format!("new Intl.NumberFormat('{locale}', {{style: 'unit', {options}}}).format({x})")
    };
    check(&[
        (&unit("en", "unit: 'kilometer'", "5"), "5 km"),
        (
            &unit("en", "unit: 'kilometer', unitDisplay: 'long'", "1"),
            "1 kilometer",
        ),
        (
            &unit("en", "unit: 'kilometer', unitDisplay: 'long'", "1.5"),
            "1.5 kilometers",
        ),
        (&unit("en", "unit: 'kilometer-per-hour'", "50"), "50 km/h"),
        (&unit("en", "unit: 'liter-per-second'", "5"), "5 L/s"),
        (
            &unit("en", "unit: 'hour', unitDisplay: 'long'", "2"),
            "2 hours",
        ),
        (
            &unit("de", "unit: 'hour', unitDisplay: 'long'", "2"),
            "2 Stunden",
        ),
        (&unit("en", "unit: 'liter'", "-3"), "-3 L"),
        (
            &unit("en", "unit: 'megabyte', unitDisplay: 'narrow'", "5"),
            "5MB",
        ),
        (&unit("en", "unit: 'percent'", "5"), "5%"),
        (&unit("en", "unit: 'celsius'", "21"), "21\u{b0}C"),
        (
            "JSON.stringify(new Intl.NumberFormat('en', {style: 'unit', unit: 'kilogram'}).formatToParts(5))",
            r#"[{"type":"integer","value":"5"},{"type":"literal","value":" "},{"type":"unit","value":"kg"}]"#,
        ),
    ]);
}

#[test]
fn compact_notation_abbreviates_as_the_locale_does() {
    let compact = |locale: &str, options: &str, x: &str| {
        format!("new Intl.NumberFormat('{locale}', {{notation: 'compact'{options}}}).format({x})")
    };
    check(&[
        (&compact("en", "", "1234"), "1.2K"),
        (&compact("en", "", "999999"), "1M"),
        (&compact("en", "", "-1234567"), "-1.2M"),
        (&compact("en", "", "999"), "999"),
        (
            &compact("en", ", compactDisplay: 'long'", "1234"),
            "1.2 thousand",
        ),
        (&compact("de", "", "1234567"), "1,2\u{a0}Mio."),
        (&compact("de", "", "12345"), "12.345"),
        (&compact("ja", "", "12345"), "1.2\u{4e07}"),
        (
            &compact("en", ", style: 'currency', currency: 'USD'", "1234"),
            "$1.2K",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('en', {notation: 'compact'}).formatToParts(1234))",
            r#"[{"type":"integer","value":"1"},{"type":"decimal","value":"."},{"type":"fraction","value":"2"},{"type":"compact","value":"K"}]"#,
        ),
    ]);
}

#[test]
fn ranges_format_as_the_reference_does() {
    check(&[
        (
            "new Intl.NumberFormat('en').formatRange(3, 5)",
            "3\u{2013}5",
        ),
        ("new Intl.NumberFormat('en').formatRange(3, 3)", "~3"),
        (
            "new Intl.NumberFormat('en').formatRange(-5, -3)",
            "-5 \u{2013} -3",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'USD', maximumFractionDigits: 0}).formatRange(3, 5)",
            "$3 \u{2013} $5",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'USD', maximumFractionDigits: 0}).formatRange(2.9, 3.1)",
            "~$3",
        ),
        (
            "new Intl.NumberFormat('es-ES', {style: 'currency', currency: 'EUR', maximumFractionDigits: 0}).formatRange(3, 5)",
            "3-5\u{a0}\u{20ac}",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('en').formatRangeToParts(3, 5))",
            "[{\"type\":\"integer\",\"value\":\"3\",\"source\":\"startRange\"},{\"type\":\"literal\",\"value\":\"\u{2013}\",\"source\":\"shared\"},{\"type\":\"integer\",\"value\":\"5\",\"source\":\"endRange\"}]",
        ),
        (
            "try { new Intl.NumberFormat('en').formatRange(1) } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "try { new Intl.NumberFormat('en').formatRange(1, NaN) } catch (e) { e.name }",
            "RangeError",
        ),
        ("new Intl.PluralRules('en').selectRange(1, 2)", "other"),
        ("new Intl.PluralRules('fr').selectRange(0, 1)", "one"),
        ("new Intl.PluralRules('sl').selectRange(102, 201)", "few"),
        (
            "try { new Intl.PluralRules('en').selectRange(1) } catch (e) { e.name }",
            "TypeError",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC'}).formatRange(Date.UTC(2020, 0, 2), Date.UTC(2020, 0, 5))",
            "1/2/2020 \u{2013} 1/5/2020",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC'}).formatRange(Date.UTC(2020, 0, 2, 3), Date.UTC(2020, 0, 2, 5))",
            "1/2/2020",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC'}).formatRangeToParts(0, 864e5).map(p => p.source).join()",
            "startRange,startRange,startRange,startRange,startRange,shared,endRange,endRange,endRange,endRange,endRange",
        ),
        (
            "try { new Intl.DateTimeFormat('en').formatRange(0) } catch (e) { e.name }",
            "TypeError",
        ),
    ]);
}

#[test]
fn numeric_dates_pick_the_pattern_the_reference_picks() {
    let at = "Date.UTC(2020, 0, 2)";
    let date = |locale: &str, options: &str| {
        format!("new Intl.DateTimeFormat('{locale}', {{timeZone: 'UTC'{options}}}).format({at})")
    };
    check(&[
        (&date("de", ""), "2.1.2020"),
        (
            &date("de", ", year: 'numeric', month: '2-digit', day: '2-digit'"),
            "02.01.2020",
        ),
        (&date("de", ", dateStyle: 'medium'"), "02.01.2020"),
        (&date("de", ", month: 'numeric', day: 'numeric'"), "2.1."),
        (&date("it", ""), "02/01/2020"),
        (&date("nl", ""), "2-1-2020"),
        (&date("ja", ""), "2020/1/2"),
        (&date("en-GB", ""), "02/01/2020"),
        (&date("fr", ""), "02/01/2020"),
        (
            &format!("new Date({at}).toLocaleDateString('de', {{timeZone: 'UTC'}})"),
            "2.1.2020",
        ),
    ]);
}

#[test]
fn iana_zones_keep_their_offsets_and_names() {
    let winter = "Date.UTC(2020, 0, 2, 15, 4, 5)";
    let summer = "Date.UTC(2020, 6, 2, 15, 4, 5)";
    let format = |locale: &str, zone: &str, options: &str, at: &str| {
        format!(
            "new Intl.DateTimeFormat('{locale}', {{timeZone: '{zone}', {options}}}).format({at})"
        )
    };
    check(&[
        (
            &format("en-US", "America/New_York", "timeStyle: 'long'", winter),
            "10:04:05 AM EST",
        ),
        (
            &format("en-US", "America/New_York", "timeStyle: 'long'", summer),
            "11:04:05 AM EDT",
        ),
        (
            &format(
                "en-US",
                "America/Los_Angeles",
                "dateStyle: 'full', timeStyle: 'full'",
                summer,
            ),
            "Thursday, July 2, 2020 at 8:04:05 AM Pacific Daylight Time",
        ),
        (
            &format("de-DE", "Europe/Berlin", "timeStyle: 'full'", summer),
            "17:04:05 Mitteleurop\u{e4}ische Sommerzeit",
        ),
        (
            &format("en-GB", "Europe/London", "timeStyle: 'long'", summer),
            "16:04:05 BST",
        ),
        (
            &format("en-US", "Asia/Kolkata", "timeStyle: 'long'", summer),
            "8:34:05 PM GMT+5:30",
        ),
        (
            &format("en-US", "Australia/Sydney", "timeStyle: 'long'", winter),
            "2:04:05 AM GMT+11",
        ),
        (
            &format("en-US", "Etc/GMT+5", "timeStyle: 'long'", summer),
            "10:04:05 AM GMT-5",
        ),
        (
            "new Intl.DateTimeFormat('en-US', {timeZone: 'asia/kolkata'}).resolvedOptions().timeZone",
            "Asia/Calcutta",
        ),
        (
            "new Intl.DateTimeFormat('en-US', {timeZone: 'US/Eastern'}).resolvedOptions().timeZone",
            "America/New_York",
        ),
        (
            "new Intl.DateTimeFormat('en-US', {timeZone: 'Etc/UTC'}).resolvedOptions().timeZone",
            "UTC",
        ),
        (
            "String(Intl.supportedValuesOf('timeZone').includes('Europe/Berlin'))",
            "true",
        ),
    ]);
}

#[test]
fn locales_tell_their_weeks_calendars_and_zones() {
    // Chrome 156's: the methods, no getters, no minimal days.
    check(&[
        (
            "JSON.stringify(new Intl.Locale('en-US').getWeekInfo())",
            r#"{"firstDay":7,"weekend":[6,7]}"#,
        ),
        (
            "JSON.stringify(new Intl.Locale('de').getWeekInfo())",
            r#"{"firstDay":1,"weekend":[6,7]}"#,
        ),
        (
            "JSON.stringify(new Intl.Locale('ar-SA').getWeekInfo())",
            r#"{"firstDay":7,"weekend":[5,6]}"#,
        ),
        (
            "new Intl.Locale('ar').getTextInfo().direction + new Intl.Locale('en').getTextInfo().direction",
            "rtlltr",
        ),
        (
            "JSON.stringify([new Intl.Locale('en-US').getHourCycles(), new Intl.Locale('ja').getHourCycles(), new Intl.Locale('en-u-hc-h23').getHourCycles()])",
            r#"[["h12"],["h23"],["h23"]]"#,
        ),
        (
            "JSON.stringify([new Intl.Locale('ja-JP').getCalendars(), new Intl.Locale('th').getCalendars(), new Intl.Locale('en').getCalendars()])",
            r#"[["gregory","japanese"],["buddhist","gregory"],["gregory"]]"#,
        ),
        (
            "JSON.stringify([new Intl.Locale('de').getCollations(), new Intl.Locale('en').getCollations()])",
            r#"[["emoji","eor","phonebk"],["emoji","eor"]]"#,
        ),
        (
            "JSON.stringify([new Intl.Locale('ar-EG').getNumberingSystems(), new Intl.Locale('en').getNumberingSystems()])",
            r#"[["arab"],["latn"]]"#,
        ),
        (
            "JSON.stringify(new Intl.Locale('de-DE').getTimeZones())",
            r#"["Europe/Berlin","Europe/Busingen"]"#,
        ),
        (
            "new Intl.Locale('en-US').getTimeZones().length + ' ' + new Intl.Locale('en-US').getTimeZones()[0]",
            "29 America/Adak",
        ),
        ("String(new Intl.Locale('en').getTimeZones())", "undefined"),
        (
            "[typeof new Intl.Locale('en').weekInfo, typeof new Intl.Locale('en').hourCycles, typeof new Intl.Locale('en').textInfo].join()",
            "undefined,undefined,undefined",
        ),
    ]);
}

#[test]
fn outputs_are_the_reference_browsers() {
    // Chrome 156's answers (en-US its default): units, currencies and
    // numbers by its data, plural ranges, collation and hour cycles of
    // languages its ICU keeps no data of, date ranges by ICU's interval
    // formats and fallbacks, zones' names by ICU's TimeZoneFormat, the
    // Buddhist and Persian calendars, and its error text.
    check(&[
        (
            "new Intl.NumberFormat('de', {style: 'unit', unit: 'kilometer-per-hour', unitDisplay: 'long'}).format(50)",
            "50 Kilometer pro Stunde",
        ),
        (
            "new Intl.NumberFormat('ja', {style: 'unit', unit: 'gigabyte', unitDisplay: 'short'}).format(3)",
            "3 GB",
        ),
        (
            "new Intl.NumberFormat('ar', {style: 'unit', unit: 'hour', unitDisplay: 'long'}).format(3)",
            "3 \u{633}\u{627}\u{639}\u{627}\u{62a}",
        ),
        (
            "new Intl.NumberFormat('ru', {style: 'unit', unit: 'kilogram', unitDisplay: 'long'}).format(5)",
            "5 \u{43a}\u{438}\u{43b}\u{43e}\u{433}\u{440}\u{430}\u{43c}\u{43c}",
        ),
        (
            "new Intl.NumberFormat('fr-CA', {style: 'unit', unit: 'meter', unitDisplay: 'short'}).format(1)",
            "1\u{a0}m",
        ),
        (
            "new Intl.NumberFormat('en-IN', {style: 'unit', unit: 'liter', unitDisplay: 'short'}).format(5)",
            "5 l",
        ),
        (
            "new Intl.NumberFormat('en-AU', {style: 'unit', unit: 'gallon', unitDisplay: 'short'}).format(2)",
            "2 gal US",
        ),
        (
            "new Intl.NumberFormat('ak', {style: 'unit', unit: 'meter', unitDisplay: 'long'}).format(2)",
            "2 meters",
        ),
        (
            "new Intl.NumberFormat('pl', {style: 'unit', unit: 'mile-per-gallon', unitDisplay: 'long'}).format(5)",
            "5 mil na galon",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'USD', currencyDisplay: 'name', maximumFractionDigits: 0}).format(1.2)",
            "1 US dollar",
        ),
        (
            "new Intl.NumberFormat('en', {style: 'currency', currency: 'USD', currencyDisplay: 'name'}).format(1)",
            "1.00 US dollars",
        ),
        (
            "new Intl.NumberFormat('sr-Latn', {style: 'currency', currency: 'USD', currencyDisplay: 'name'}).format(2)",
            "2,00 ameri\u{10d}kih dolara",
        ),
        (
            "new Intl.NumberFormat('az', {style: 'currency', currency: 'USD'}).format(1234.5)",
            "$\u{a0}1,234.50",
        ),
        (
            "new Intl.NumberFormat('es-MX', {style: 'currency', currency: 'EGP', currencyDisplay: 'narrowSymbol'}).format(1234.5)",
            "E\u{a3}1,234.50",
        ),
        (
            "new Intl.NumberFormat('en', {maximumFractionDigits: 2}).format(1.005)",
            "1.01",
        ),
        (
            "new Intl.NumberFormat('de', {style: 'percent'}).format(0.25)",
            "25\u{a0}%",
        ),
        (
            "new Intl.NumberFormat('en', {notation: 'compact'}).formatRange(1000, 5000)",
            "1K \u{2013} 5K",
        ),
        ("new Intl.PluralRules('he').selectRange(0.5, 1)", "other"),
        ("new Intl.PluralRules('kok').selectRange(0, 1)", "other"),
        ("new Intl.PluralRules('en').selectRange(1, 2)", "other"),
        (
            "['a', 'b', 'ch', 'c', 'h'].sort(new Intl.Collator('uz').compare).join()",
            "a,b,c,ch,h",
        ),
        (
            "JSON.stringify(new Intl.Locale('pa-Arab').getHourCycles())",
            "[\"h12\"]",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC', month: 'long', day: 'numeric', year: 'numeric'}).formatRange(Date.UTC(2020, 0, 2), Date.UTC(2020, 0, 5))",
            "January 2 \u{2013} 5, 2020",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC', hour: 'numeric', minute: 'numeric'}).formatRange(Date.UTC(2020, 0, 2, 10), Date.UTC(2020, 0, 2, 15))",
            "10:00 AM \u{2013} 3:00 PM",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC', hour: 'numeric', minute: 'numeric'}).formatRange(Date.UTC(2020, 0, 2, 10), Date.UTC(2020, 0, 5, 10))",
            "1/2/2020, 10:00 AM \u{2013} 1/5/2020, 10:00 AM",
        ),
        (
            "new Intl.DateTimeFormat('ca', {timeZone: 'UTC', year: 'numeric', month: 'short'}).formatRange(Date.UTC(2020, 0, 2), Date.UTC(2020, 2, 5))",
            "gen. del 2020 \u{2013} mar\u{e7} del 2020",
        ),
        (
            "new Intl.DateTimeFormat('fa', {timeZone: 'UTC', year: 'numeric', month: 'short'}).formatRange(Date.UTC(2020, 5, 2), Date.UTC(2020, 7, 25))",
            "\u{62e}\u{631}\u{62f}\u{627}\u{62f} \u{6f1}\u{6f3}\u{6f9}\u{6f9}",
        ),
        (
            "new Intl.DateTimeFormat('cs', {timeZone: 'UTC', dateStyle: 'short', timeStyle: 'short'}).formatRange(Date.UTC(2020, 0, 2, 10), Date.UTC(2020, 0, 2, 11, 30))",
            "02. 01. 20 10:00\u{2013}11:30",
        ),
        (
            "new Intl.DateTimeFormat('zh-Hant', {timeZone: 'UTC', dateStyle: 'short', timeStyle: 'short'}).formatRange(Date.UTC(2020, 0, 2, 10), Date.UTC(2020, 0, 2, 15))",
            "2020/1/2 \u{4e0a}\u{5348}10:00 \u{2013} \u{4e0b}\u{5348}3:00",
        ),
        (
            "new Intl.DateTimeFormat('af', {timeZone: 'UTC', month: 'long', day: 'numeric', hour: 'numeric', minute: '2-digit'}).formatRange(Date.UTC(2020, 0, 2, 10), Date.UTC(2021, 2, 5, 10))",
            "2 Januarie 2020 om 10:00 \u{2013} 5 Maart 2021 om 10:00",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'Asia/Tokyo', timeZoneName: 'longGeneric'}).format(Date.UTC(2020, 6, 2))",
            "7/2/2020, Japan Standard Time",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'Asia/Hong_Kong', timeZoneName: 'shortGeneric'}).format(Date.UTC(2020, 6, 2))",
            "7/2/2020, Hong Kong Time",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'Europe/London', timeZoneName: 'longGeneric'}).format(Date.UTC(2020, 6, 2))",
            "7/2/2020, United Kingdom Time",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'Europe/Dublin', timeZoneName: 'longGeneric'}).format(Date.UTC(2020, 6, 2))",
            "7/2/2020, Ireland Time",
        ),
        (
            "new Intl.DateTimeFormat('de', {timeZone: 'Africa/Cairo', timeZoneName: 'shortGeneric'}).format(Date.UTC(2020, 6, 2))",
            "2.7.2020, OEZ (\u{c4}gypten)",
        ),
        (
            "new Intl.DateTimeFormat('ja', {timeZone: 'America/New_York', timeZoneName: 'shortGeneric'}).format(Date.UTC(2020, 6, 2))",
            "2020/7/1 New York\u{6642}\u{9593}",
        ),
        (
            "new Intl.DateTimeFormat('ko', {timeZone: 'America/New_York', timeZoneName: 'long'}).format(Date.UTC(2020, 6, 2))",
            "2020. 7. 1. \u{bd81}\u{bbf8} \u{b3d9}\u{bd80} \u{d558}\u{acc4} \u{d45c}\u{c900}\u{c2dc}",
        ),
        (
            "new Intl.DateTimeFormat('de', {timeZone: 'Europe/Istanbul', timeZoneName: 'long'}).format(Date.UTC(2020, 6, 2))",
            "2.7.2020, T\u{fc}rkische Normalzeit",
        ),
        (
            "new Intl.DateTimeFormat('bs-Cyrl', {timeZone: 'Asia/Tokyo', timeZoneName: 'shortGeneric'}).format(Date.UTC(2020, 6, 2))",
            "02.07.2020. JP",
        ),
        (
            "new Intl.DateTimeFormat('fr', {timeZone: 'UTC', timeZoneName: 'shortOffset'}).format(Date.UTC(2020, 6, 2))",
            "02/07/2020 UTC+0",
        ),
        (
            "new Intl.DateTimeFormat('he', {timeZone: 'America/Los_Angeles', timeZoneName: 'short'}).format(Date.UTC(2020, 6, 2))",
            "1.7.2020, GMT-7\u{200e}",
        ),
        (
            "new Intl.DateTimeFormat('cs', {timeZone: 'UTC', timeZoneName: 'longOffset'}).format(Date.UTC(2020, 6, 2))",
            "2. 7. 2020 GMT+00:00",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'Africa/Casablanca', timeZoneName: 'longOffset'}).format(Date.UTC(2025, 2, 1))",
            "3/1/2025, GMT+00:00",
        ),
        (
            "new Intl.DateTimeFormat('th', {timeZone: 'UTC', dateStyle: 'full'}).format(Date.UTC(2020, 0, 2))",
            "\u{e27}\u{e31}\u{e19}\u{e1e}\u{e24}\u{e2b}\u{e31}\u{e2a}\u{e1a}\u{e14}\u{e35}\u{e17}\u{e35}\u{e48} 2 \u{e21}\u{e01}\u{e23}\u{e32}\u{e04}\u{e21} \u{e1e}.\u{e28}. 2563",
        ),
        (
            "new Intl.DateTimeFormat('fa', {timeZone: 'UTC', dateStyle: 'full'}).format(Date.UTC(2020, 0, 2))",
            "\u{6f1}\u{6f3}\u{6f9}\u{6f8} \u{62f}\u{6cc} \u{6f1}\u{6f2}, \u{67e}\u{646}\u{62c}\u{634}\u{646}\u{628}\u{647}",
        ),
        (
            "new Intl.DateTimeFormat('pa-Arab', {timeZone: 'UTC', month: 'long'}).format(Date.UTC(2020, 0, 2))",
            "\u{62c}\u{646}\u{648}\u{631}\u{6cc}",
        ),
        (
            "new Intl.DateTimeFormat('cs', {timeZone: 'UTC', era: 'short'}).format(Date.UTC(2020, 0, 2))",
            "2. 1. 2020 n. l.",
        ),
        (
            "new Intl.DateTimeFormat('de', {timeZone: 'UTC', second: 'numeric', fractionalSecondDigits: 2}).format(Date.UTC(2020, 0, 2, 0, 0, 5))",
            "5,00",
        ),
        (
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC', dayPeriod: 'long'}).format(Date.UTC(2020, 0, 2, 15))",
            "in the afternoon",
        ),
        (
            "new Intl.DateTimeFormat('de', {timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric'}).format(Date.UTC(2020, 0, 2))",
            "2.1.2020",
        ),
        (
            "try { Intl.getCanonicalLocales('en_US') } catch (e) { e.message }",
            "Invalid language tag: en_US",
        ),
    ]);
}

#[test]
fn segmenter_breaks_and_answers_as_the_reference() {
    // The reference browser's (Chrome 156) answers.
    check(&[
        (
            "(() => { try { Intl.Segmenter(); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Constructor Intl.Segmenter requires 'new'",
        ),
        (
            "(() => { try { new Intl.Segmenter('en', 5); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.Segmenter('en', {granularity: 'line'}); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Value line out of range for Intl.Segmenter options property granularity",
        ),
        (
            "(() => { try { Intl.Segmenter.prototype.segment.call({}, 'a'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Method Intl.Segmenter.prototype.segment called on incompatible receiver #<Object>",
        ),
        (
            "(() => { const s = new Intl.Segmenter('en').segment('ab'); try { s.containing.call({}, 0); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Method %Segments.prototype%.containing called on incompatible receiver #<Object>",
        ),
        (
            "(() => { const it = new Intl.Segmenter('en').segment('ab')[Symbol.iterator](); try { it.next.call({}); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Method %SegmentIterator.prototype%.next called on incompatible receiver #<Object>",
        ),
        (
            "JSON.stringify(new Intl.Segmenter('fr-u-ca-buddhist', {granularity: 'word'}).resolvedOptions())",
            "{\"locale\":\"fr\",\"granularity\":\"word\"}",
        ),
        (
            "JSON.stringify(new Intl.Segmenter().resolvedOptions())",
            "{\"locale\":\"en-US\",\"granularity\":\"grapheme\"}",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en', {granularity: 'word'}).segment('Hello, world! 3.14 is pi. Don\\'t stop')])",
            "[{\"segment\":\"Hello\",\"index\":0,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\",\",\"index\":5,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\" \",\"index\":6,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"world\",\"index\":7,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\"!\",\"index\":12,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\" \",\"index\":13,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"3.14\",\"index\":14,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\" \",\"index\":18,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"is\",\"index\":19,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\" \",\"index\":21,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"pi\",\"index\":22,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\".\",\"index\":24,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\" \",\"index\":25,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"Don't\",\"index\":26,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true},{\"segment\":\" \",\"index\":31,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":false},{\"segment\":\"stop\",\"index\":32,\"input\":\"Hello, world! 3.14 is pi. Don't stop\",\"isWordLike\":true}]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en', {granularity: 'sentence'}).segment('Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...')])",
            "[{\"segment\":\"Hello, world! \",\"index\":0,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"},{\"segment\":\"3.14 is pi. \",\"index\":14,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"},{\"segment\":\"Mr. \",\"index\":26,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"},{\"segment\":\"Smith went home.  \",\"index\":30,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"},{\"segment\":\"Yes? \",\"index\":48,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"},{\"segment\":\"No...\",\"index\":53,\"input\":\"Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...\"}]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en').segment('e\u{301}\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{1f1eb}\u{1f1f7}a\\r\\nb')].map(s => s.index))",
            "[0,2,10,14,15,17]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('ja', {granularity: 'word'}).segment('\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{4e16}\u{754c}\u{3002}\u{4eca}\u{65e5}\u{306f}\u{6674}\u{308c}\u{3067}\u{3059}')].map(s => s.segment + '|' + s.isWordLike))",
            "[\"\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}|true\",\"\u{4e16}\u{754c}|true\",\"\u{3002}|false\",\"\u{4eca}\u{65e5}|true\",\"\u{306f}|true\",\"\u{6674}\u{308c}|true\",\"\u{3067}\u{3059}|true\"]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('th', {granularity: 'word'}).segment('\u{e17}\u{e38}\u{e01}\u{e2a}\u{e2d}\u{e07}\u{e2a}\u{e31}\u{e1b}\u{e14}\u{e32}\u{e2b}\u{e4c}')].map(s => s.segment + '|' + s.isWordLike))",
            "[\"\u{e17}\u{e38}\u{e01}|true\",\"\u{e2a}\u{e2d}\u{e07}|true\",\"\u{e2a}\u{e31}\u{e1b}\u{e14}\u{e32}\u{e2b}\u{e4c}|true\"]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('zh', {granularity: 'word'}).segment('\u{6211}\u{4eec}\u{662f}\u{4e2d}\u{56fd}\u{4eba}\u{3002}')].map(s => s.segment + '|' + s.isWordLike))",
            "[\"\u{6211}\u{4eec}|true\",\"\u{662f}|true\",\"\u{4e2d}\u{56fd}\u{4eba}|true\",\"\u{3002}|false\"]",
        ),
        (
            "JSON.stringify(new Intl.Segmenter('en', {granularity: 'word'}).segment('abc def').containing(5))",
            "{\"segment\":\"def\",\"index\":4,\"input\":\"abc def\",\"isWordLike\":true}",
        ),
        (
            "JSON.stringify([new Intl.Segmenter('en').segment('abc').containing(-1), new Intl.Segmenter('en').segment('abc').containing(3), new Intl.Segmenter('en').segment('abc').containing('1').index, new Intl.Segmenter('en').segment('abc').containing().index, new Intl.Segmenter('en').segment('abc').containing(1.9).index])",
            "[null,null,1,0,1]",
        ),
        (
            "Object.prototype.toString.call(new Intl.Segmenter('en').segment('a')[Symbol.iterator]())",
            "[object Segmenter String Iterator]",
        ),
        (
            "Object.prototype.toString.call(new Intl.Segmenter('en').segment('a'))",
            "[object Object]",
        ),
        (
            "Object.getOwnPropertyNames(Object.getPrototypeOf(new Intl.Segmenter('en').segment('a'))).join()",
            "containing",
        ),
        (
            "Object.getOwnPropertyNames(Object.getPrototypeOf(new Intl.Segmenter('en').segment('a')[Symbol.iterator]())).join()",
            "next",
        ),
        (
            "String(Object.getPrototypeOf(Object.getPrototypeOf(new Intl.Segmenter('en').segment('a')[Symbol.iterator]())) === Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())))",
            "true",
        ),
        (
            "Object.getOwnPropertyNames(Intl.Segmenter.prototype).join()",
            "constructor,resolvedOptions,segment",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en').segment('\\ud800a\\udc00')].map(s => s.segment.length))",
            "[1,1,1]",
        ),
        (
            "JSON.stringify(Intl.Segmenter.supportedLocalesOf(['en', 'xx', 'de-AT']))",
            "[\"en\",\"de-AT\"]",
        ),
        (
            "String(Intl.Segmenter.length) + Intl.Segmenter.name + Intl.Segmenter.prototype.segment.length",
            "0Segmenter1",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en', {granularity: 'word'}).segment('a_b 12,345 x.y e-mail \u{e9}t\u{e9} can\\'t \u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629} \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}')].map(s => s.segment + '|' + s.isWordLike))",
            "[\"a_b|true\",\" |false\",\"12,345|true\",\" |false\",\"x|true\",\".|false\",\"y|true\",\" |false\",\"e|true\",\"-|false\",\"mail|true\",\" |false\",\"\u{e9}t\u{e9}|true\",\" |false\",\"can't|true\",\" |false\",\"\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629}|true\",\" |false\",\"\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}|true\"]",
        ),
        (
            "JSON.stringify(Object.keys(new Intl.Segmenter('en', {granularity: 'word'}).segment('ab').containing(0)))",
            "[\"segment\",\"index\",\"input\",\"isWordLike\"]",
        ),
        (
            "JSON.stringify(Object.keys(new Intl.Segmenter('en').segment('ab').containing(0)))",
            "[\"segment\",\"index\",\"input\"]",
        ),
        (
            "JSON.stringify([...new Intl.Segmenter('en', {granularity: 'sentence'}).segment('He said \"Hi.\" Then left.\\nNew line here\u{2029}Para two. 1. item')].map(s => s.segment))",
            "[\"He said \\\"Hi.\\\" \",\"Then left.\\n\",\"New line here\u{2029}\",\"Para two. \",\"1. item\"]",
        ),
    ]);
}

#[test]
fn duration_format_writes_as_the_reference() {
    // The reference browser's (Chrome 156) answers, a sample of 2,080.
    check(&[
        (
            "(() => { try { return String(Intl.DurationFormat()); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Constructor Intl.DurationFormat requires 'new'",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', 5)); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: invalid_argument",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {style: 'x'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Value x out of range for Intl.DurationFormat options property style",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {hours: 'x'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Value x out of range for Intl.DurationFormat options property hours",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {millisecondsDisplay: 'always', milliseconds: 'numeric', seconds: 'numeric'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Invalid object : #<Object>",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {hours: 'numeric', minutes: 'long'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Invalid object : #<Object>",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {fractionalDigits: 10})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: fractionalDigits value is out of range.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en', {numberingSystem: 'x'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Invalid numberingSystem : x",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format()); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Temporal error: Duration argument must be Duration or string.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format('PT1H')); } catch (e) { return e.name + ': ' + e.message; } })()",
            "1 hr",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Temporal error: Did not provide any valid Duration fields.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({hours: 1, minutes: -1})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Temporal error: Duration was not valid.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({hours: 1.5})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Temporal error: Expected finite integer.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({years: 2**32})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Temporal error: Duration was not valid.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({seconds: 2**53})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Temporal error: Duration was not valid.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({hours: Infinity})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Temporal error: Expected finite integer.",
        ),
        (
            "(() => { try { return String(new Intl.DurationFormat('en').format({hours: '2'})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "2 hr",
        ),
        (
            "(() => { try { return String(Intl.DurationFormat.prototype.format.call({}, {hours: 1})); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Method Intl.DurationFormat.prototype.format called on incompatible receiver #<Object>",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en').resolvedOptions())",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"short\",\"years\":\"short\",\"yearsDisplay\":\"auto\",\"months\":\"short\",\"monthsDisplay\":\"auto\",\"weeks\":\"short\",\"weeksDisplay\":\"auto\",\"days\":\"short\",\"daysDisplay\":\"auto\",\"hours\":\"short\",\"hoursDisplay\":\"auto\",\"minutes\":\"short\",\"minutesDisplay\":\"auto\",\"seconds\":\"short\",\"secondsDisplay\":\"auto\",\"milliseconds\":\"short\",\"millisecondsDisplay\":\"auto\",\"microseconds\":\"short\",\"microsecondsDisplay\":\"auto\",\"nanoseconds\":\"short\",\"nanosecondsDisplay\":\"auto\"}",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr-u-nu-arab-ca-buddhist', {style: 'digital', fractionalDigits: 3}).resolvedOptions())",
            "{\"locale\":\"fr-u-nu-arab\",\"numberingSystem\":\"arab\",\"style\":\"digital\",\"years\":\"short\",\"yearsDisplay\":\"auto\",\"months\":\"short\",\"monthsDisplay\":\"auto\",\"weeks\":\"short\",\"weeksDisplay\":\"auto\",\"days\":\"short\",\"daysDisplay\":\"auto\",\"hours\":\"numeric\",\"hoursDisplay\":\"always\",\"minutes\":\"2-digit\",\"minutesDisplay\":\"always\",\"seconds\":\"2-digit\",\"secondsDisplay\":\"always\",\"milliseconds\":\"numeric\",\"millisecondsDisplay\":\"auto\",\"microseconds\":\"numeric\",\"microsecondsDisplay\":\"auto\",\"nanoseconds\":\"numeric\",\"nanosecondsDisplay\":\"auto\",\"fractionalDigits\":3}",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {style: 'long', hours: 'numeric'}).resolvedOptions())",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"long\",\"years\":\"long\",\"yearsDisplay\":\"auto\",\"months\":\"long\",\"monthsDisplay\":\"auto\",\"weeks\":\"long\",\"weeksDisplay\":\"auto\",\"days\":\"long\",\"daysDisplay\":\"auto\",\"hours\":\"numeric\",\"hoursDisplay\":\"always\",\"minutes\":\"2-digit\",\"minutesDisplay\":\"always\",\"seconds\":\"2-digit\",\"secondsDisplay\":\"always\",\"milliseconds\":\"numeric\",\"millisecondsDisplay\":\"auto\",\"microseconds\":\"numeric\",\"microsecondsDisplay\":\"auto\",\"nanoseconds\":\"numeric\",\"nanosecondsDisplay\":\"auto\"}",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {seconds: 'numeric'}).resolvedOptions())",
            "{\"locale\":\"en\",\"numberingSystem\":\"latn\",\"style\":\"short\",\"years\":\"short\",\"yearsDisplay\":\"auto\",\"months\":\"short\",\"monthsDisplay\":\"auto\",\"weeks\":\"short\",\"weeksDisplay\":\"auto\",\"days\":\"short\",\"daysDisplay\":\"auto\",\"hours\":\"short\",\"hoursDisplay\":\"auto\",\"minutes\":\"short\",\"minutesDisplay\":\"auto\",\"seconds\":\"numeric\",\"secondsDisplay\":\"always\",\"milliseconds\":\"numeric\",\"millisecondsDisplay\":\"auto\",\"microseconds\":\"numeric\",\"microsecondsDisplay\":\"auto\",\"nanoseconds\":\"numeric\",\"nanosecondsDisplay\":\"auto\"}",
        ),
        (
            "Object.getOwnPropertyNames(Intl.DurationFormat.prototype).join()",
            "constructor,resolvedOptions,format,formatToParts",
        ),
        (
            "Intl.DurationFormat.length + ' ' + Intl.DurationFormat.prototype.format.length",
            "0 1",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {}).format({minutes: 1, nanoseconds: 1}))",
            "\"1 min, 1 ns\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {style: 'digital'}).format({hours: 0}))",
            "\"0:00:00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {hours: 'numeric'}).format({minutes: 61}))",
            "\"0:61:00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {style: 'short', secondsDisplay: 'always'}).format({nanoseconds: 1}))",
            "\"0 sec, 1 ns\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long'}).format({hours: 1, minutes: 0, seconds: 3}))",
            "\"1 Stunde, 3 Sekunden\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).format({seconds: -0.0, milliseconds: -1}))",
            "\"-0:00:00,001\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {hours: 'numeric'}).format({days: -2, hours: -3}))",
            "\"-2 Tg., 3:00:00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', daysDisplay: 'always', hoursDisplay: 'always'}).format({hours: 1, minutes: 2, seconds: 3}))",
            "\"0 Tage, 1 Stunde, 2 Minuten und 3 Sekunden\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {milliseconds: 'numeric'}).format({hours: 10, minutes: 5, seconds: 2, milliseconds: 30}))",
            "\"10 Std., 5 Min. und 2,03 Sek.\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr', {style: 'long'}).format({milliseconds: 1234, microseconds: 5}))",
            "\"1\u{202f}234\u{a0}millisecondes et 5\u{a0}microsecondes\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr', {style: 'digital', fractionalDigits: 2}).format({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "\"1\u{a0}an, 2\u{202f}m., 3\u{202f}sem., 4\u{202f}j et 5:06:07,00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr', {hours: 'numeric'}).format({years: 1000000, seconds: 1}))",
            "\"1\u{202f}000\u{202f}000\u{a0}ans et 0:00:01\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr', {style: 'long', daysDisplay: 'always', hoursDisplay: 'always'}).format({seconds: 1, milliseconds: 500}))",
            "\"0\u{a0}jour, 0\u{a0}heure, 1\u{a0}seconde et 500\u{a0}millisecondes\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fr', {milliseconds: 'numeric'}).format({minutes: 1, nanoseconds: 1}))",
            "\"1\u{a0}min et 0,000000001\u{202f}s\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ja', {style: 'long'}).format({hours: 0}))",
            "\"\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ja', {style: 'digital', fractionalDigits: 2}).format({minutes: 61}))",
            "\"0:61:00.00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ja', {hours: 'numeric'}).format({nanoseconds: 1}))",
            "\"0:00:00.000000001\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ja', {style: 'digital', hoursDisplay: 'auto'}).format({hours: 1, minutes: 0, seconds: 3}))",
            "\"1:00:03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long'}).format({seconds: -0.0, milliseconds: -1}))",
            "\"\u{200e}-1 \u{645}\u{644}\u{64a} \u{62b}\u{627}\u{646}\u{64a}\u{629}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital', fractionalDigits: 2}).format({days: -2, hours: -3}))",
            "\"\u{64a}\u{648}\u{645}\u{627}\u{646} \u{648}3:00:00.00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {hours: '2-digit', minutes: 'numeric'}).format({hours: 1, minutes: 2, seconds: 3}))",
            "\"01:02:03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', daysDisplay: 'always', hoursDisplay: 'always'}).format({hours: 10, minutes: 5, seconds: 2, milliseconds: 30}))",
            "\"0 \u{64a}\u{648}\u{645}\u{60c} \u{648}10 \u{633}\u{627}\u{639}\u{627}\u{62a}\u{60c} \u{648}5 \u{62f}\u{642}\u{627}\u{626}\u{642}\u{60c} \u{648}\u{62b}\u{627}\u{646}\u{64a}\u{62a}\u{627}\u{646}\u{60c} \u{648}30 \u{645}\u{644}\u{64a} \u{62b}\u{627}\u{646}\u{64a}\u{629}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital', hoursDisplay: 'auto'}).format({milliseconds: 1234, microseconds: 5}))",
            "\"00:01.234005\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ru', {style: 'narrow'}).format({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "\"1 \u{433}. 2 \u{43c}. 3 \u{43d}. 4 \u{434}. 5 \u{447} 6 \u{43c}\u{438}\u{43d} 7 \u{441} 8 \u{43c}\u{441} 9 \u{43c}\u{43a}\u{441} 10 \u{43d}\u{441}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ru', {style: 'digital', fractionalDigits: 2}).format({years: 1000000, seconds: 1}))",
            "\"1\u{a0}000\u{a0}000 \u{43b}. 0:00:01,00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ru', {hours: '2-digit', minutes: 'numeric'}).format({seconds: 1, milliseconds: 500}))",
            "\"00:00:01,5\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ru', {style: 'long', daysDisplay: 'always', hoursDisplay: 'always'}).format({minutes: 1, nanoseconds: 1}))",
            "\"0 \u{434}\u{43d}\u{435}\u{439} 0 \u{447}\u{430}\u{441}\u{43e}\u{432} 1 \u{43c}\u{438}\u{43d}\u{443}\u{442}\u{430} 1 \u{43d}\u{430}\u{43d}\u{43e}\u{441}\u{435}\u{43a}\u{443}\u{43d}\u{434}\u{430}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ru', {style: 'digital', hoursDisplay: 'auto'}).format({hours: 0}))",
            "\"00:00\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fa', {style: 'narrow'}).format({minutes: 61}))",
            "\"\u{6f6}\u{6f1}m\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fa', {style: 'digital', fractionalDigits: 2}).format({nanoseconds: 1}))",
            "\"\u{6f0}:\u{6f0}\u{6f0}:\u{6f0}\u{6f0}\u{66b}\u{6f0}\u{6f0}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fa', {seconds: 'numeric', fractionalDigits: 0}).format({hours: 1, minutes: 0, seconds: 3}))",
            "\"\u{6f1} \u{633}\u{627}\u{639}\u{62a}:\u{6f3}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fa', {style: 'digital', hoursDisplay: 'auto'}).format({seconds: -0.0, milliseconds: -1}))",
            "\"\u{200e}\u{2212}\u{6f0}\u{6f0}:\u{6f0}\u{6f0}\u{66b}\u{6f0}\u{6f0}\u{6f1}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('zh', {style: 'narrow'}).format({days: -2, hours: -3}))",
            "\"-2\u{5929}3\u{5c0f}\u{65f6}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('zh', {style: 'long', minutes: 'numeric'}).format({hours: 1, minutes: 2, seconds: 3}))",
            "\"1\u{5c0f}\u{65f6}2:03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('zh', {hours: '2-digit', minutes: 'numeric'}).format({hours: 10, minutes: 5, seconds: 2, milliseconds: 30}))",
            "\"10:05:02.03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('zh', {seconds: 'numeric', fractionalDigits: 0}).format({milliseconds: 1234, microseconds: 5}))",
            "\"1\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('es', {}).format({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "\"1 a, 2 m., 3 sem., 4 d, 5 h, 6 min, 7 s, 8 ms, 9 \u{3bc}s, 10 ns\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('es', {style: 'narrow'}).format({years: 1000000, seconds: 1}))",
            "\"1.000.000a 1s\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('es', {style: 'long', minutes: 'numeric'}).format({seconds: 1, milliseconds: 500}))",
            "\"0:01,5\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('es', {hours: '2-digit', minutes: 'numeric'}).format({minutes: 1, nanoseconds: 1}))",
            "\"00:01:00,000000001\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('es', {seconds: 'numeric', fractionalDigits: 0}).format({hours: 0}))",
            "\"0\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('hi', {}).format({minutes: 61}))",
            "\"61 \u{92e}\u{93f}\u{970}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('hi', {style: 'narrow'}).format({nanoseconds: 1}))",
            "\"1 \u{928}\u{948}\u{928}\u{94b} \u{938}\u{947}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('hi', {style: 'short', secondsDisplay: 'always'}).format({hours: 1, minutes: 0, seconds: 3}))",
            "\"1 \u{918}\u{902}\u{970}, 3 \u{938}\u{947}\u{970}\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('hi', {seconds: 'numeric', fractionalDigits: 0}).format({seconds: -0.0, milliseconds: -1}))",
            "\"-0\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fi', {}).format({days: -2, hours: -3}))",
            "\"\u{2212}2 pv, 3 t\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fi', {style: 'digital'}).format({hours: 1, minutes: 2, seconds: 3}))",
            "\"1.02.03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fi', {style: 'long', minutes: 'numeric'}).format({hours: 10, minutes: 5, seconds: 2, milliseconds: 30}))",
            "\"10 tuntia ja 5.02,03\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fi', {style: 'short', secondsDisplay: 'always'}).format({milliseconds: 1234, microseconds: 5}))",
            "\"0 s, 1\u{a0}234 ms, 5 \u{3bc}s\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('fi', {milliseconds: 'numeric'}).format({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "\"1 v, 2 kk, 3 vk, 4 pv, 5 t, 6 min, 7,00800901 s\"",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('en', {style: 'digital'}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({days: -2, hours: -3}))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"year\"},{\"type\":\"unit\",\"value\":\"J\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"month\"},{\"type\":\"unit\",\"value\":\"Mon.\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"week\"},{\"type\":\"unit\",\"value\":\"Wo.\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"4\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"Tg.\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" und \"},{\"type\":\"integer\",\"value\":\"5\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"06\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"07\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\",\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"00800901\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({hours: 1, minutes: 2, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"02\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({hours: 1, minutes: 0, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({days: -2, hours: -3}))",
            "[{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"day\"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"Tg.\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'digital'}).formatToParts({seconds: -0.0, milliseconds: -1}))",
            "[{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"hour\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\",\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"001\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"year\"},{\"type\":\"unit\",\"value\":\"Jahr\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"month\"},{\"type\":\"unit\",\"value\":\"Monate\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"week\"},{\"type\":\"unit\",\"value\":\"Wochen\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"4\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"Tage\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"5\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"Stunden\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" und \"},{\"type\":\"integer\",\"value\":\"6\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"07\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\",\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"00800901\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({hours: 1, minutes: 2, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"Stunde\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({hours: 1, minutes: 0, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"Stunde\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({days: -2, hours: -3}))",
            "[{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"day\"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"Tage\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"Stunden\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" und \"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('de', {style: 'long', minutes: 'numeric'}).formatToParts({seconds: -0.0, milliseconds: -1}))",
            "[{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"minute\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\",\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"001\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{646}\u{629} \u{648}\u{627}\u{62d}\u{62f}\u{629}\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"unit\",\"value\":\"\u{634}\u{647}\u{631}\u{627}\u{646}\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"week\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{633}\u{627}\u{628}\u{64a}\u{639}\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"4\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{64a}\u{627}\u{645}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"5\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"6\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"minute\"},{\"type\":\"unit\",\"value\":\"\u{62f}\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"7\",\"unit\":\"second\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"second\"},{\"type\":\"unit\",\"value\":\"\u{62b}\",\"unit\":\"second\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"8\",\"unit\":\"millisecond\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"millisecond\"},{\"type\":\"unit\",\"value\":\"\u{645}\u{644}\u{64a} \u{62b}\",\"unit\":\"millisecond\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"9\",\"unit\":\"microsecond\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"microsecond\"},{\"type\":\"unit\",\"value\":\"\u{645}.\u{62b}.\",\"unit\":\"microsecond\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"10\",\"unit\":\"nanosecond\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"nanosecond\"},{\"type\":\"unit\",\"value\":\"\u{646}.\u{62b}.\",\"unit\":\"nanosecond\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({hours: 1, minutes: 2, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"minute\"},{\"type\":\"unit\",\"value\":\"\u{62f}\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"second\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"second\"},{\"type\":\"unit\",\"value\":\"\u{62b}\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({hours: 1, minutes: 0, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"second\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"second\"},{\"type\":\"unit\",\"value\":\"\u{62b}\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"minute\"},{\"type\":\"unit\",\"value\":\"\u{62f}\",\"unit\":\"minute\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({days: -2, hours: -3}))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\",\"unit\":\"hour\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {}).formatToParts({seconds: -0.0, milliseconds: -1}))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\",\"unit\":\"millisecond\"},{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"millisecond\"},{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"millisecond\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"millisecond\"},{\"type\":\"unit\",\"value\":\"\u{645}\u{644}\u{64a} \u{62b}\",\"unit\":\"millisecond\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{646}\u{629} \u{648}\u{627}\u{62d}\u{62f}\u{629}\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"unit\",\"value\":\"\u{634}\u{647}\u{631}\u{627}\u{646}\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"week\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{633}\u{627}\u{628}\u{64a}\u{639}\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"4\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{64a}\u{627}\u{645}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"5\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"06\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"07\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\".\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"00800901\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({hours: 1, minutes: 2, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"02\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({hours: 1, minutes: 0, seconds: 3}))",
            "[{\"type\":\"integer\",\"value\":\"1\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'digital'}).formatToParts({seconds: -0.0, milliseconds: -1}))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\",\"unit\":\"hour\"},{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"hour\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\".\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"001\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({years: 1, months: 2, weeks: 3, days: 4, hours: 5, minutes: 6, seconds: 7, milliseconds: 8, microseconds: 9, nanoseconds: 10}))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{646}\u{629}\",\"unit\":\"year\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"unit\",\"value\":\"\u{634}\u{647}\u{631}\u{627}\u{646}\",\"unit\":\"month\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"week\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{633}\u{627}\u{628}\u{64a}\u{639}\",\"unit\":\"week\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"4\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"day\"},{\"type\":\"unit\",\"value\":\"\u{623}\u{64a}\u{627}\u{645}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"5\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\u{627}\u{639}\u{627}\u{62a}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"6\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"07\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\".\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"00800901\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({hours: 1, minutes: 2, seconds: 3}))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{627}\u{639}\u{629}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \u{648}\"},{\"type\":\"integer\",\"value\":\"2\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({hours: 1, minutes: 0, seconds: 3}))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{627}\u{639}\u{629}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \u{648}\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"03\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({minutes: 61}))",
            "[{\"type\":\"integer\",\"value\":\"61\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({days: -2, hours: -3}))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\",\"unit\":\"day\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"3\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\" \",\"unit\":\"hour\"},{\"type\":\"unit\",\"value\":\"\u{633}\u{627}\u{639}\u{627}\u{62a}\",\"unit\":\"hour\"},{\"type\":\"literal\",\"value\":\"\u{60c} \u{648}\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"}]",
        ),
        (
            "JSON.stringify(new Intl.DurationFormat('ar', {style: 'long', minutes: 'numeric'}).formatToParts({seconds: -0.0, milliseconds: -1}))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\",\"unit\":\"minute\"},{\"type\":\"minusSign\",\"value\":\"-\",\"unit\":\"minute\"},{\"type\":\"integer\",\"value\":\"0\",\"unit\":\"minute\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"integer\",\"value\":\"00\",\"unit\":\"second\"},{\"type\":\"decimal\",\"value\":\".\",\"unit\":\"second\"},{\"type\":\"fraction\",\"value\":\"001\",\"unit\":\"second\"}]",
        ),
    ]);
}

#[test]
fn number_signs_and_words_as_the_reference() {
    // Direction marks apart from signs; a unit written in words has no sign (Chrome 156).
    check(&[
        (
            "JSON.stringify(new Intl.NumberFormat('ar').formatToParts(-1))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"1\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'long'}).formatToParts(-2))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'short'}).formatToParts(-2))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'long'}).formatToParts(2))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'hour', unitDisplay: 'long'}).formatToParts(-1))",
            "[{\"type\":\"unit\",\"value\":\"\u{633}\u{627}\u{639}\u{629}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('fa').formatToParts(-1))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"\u{2212}\"},{\"type\":\"integer\",\"value\":\"\u{6f1}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('he').formatToParts(-1))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"1\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ur').formatToParts(-1))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"1\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {signDisplay: 'always'}).formatToParts(1))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"plusSign\",\"value\":\"+\"},{\"type\":\"integer\",\"value\":\"1\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar-u-nu-arab').formatToParts(-1))",
            "[{\"type\":\"literal\",\"value\":\"\u{61c}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"\u{661}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'percent'}).formatToParts(-0.5))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"50\"},{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"percentSign\",\"value\":\"%\"},{\"type\":\"literal\",\"value\":\"\u{200e}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'currency', currency: 'EGP'}).formatToParts(-5))",
            "[{\"type\":\"literal\",\"value\":\"\u{200f}\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"5\"},{\"type\":\"decimal\",\"value\":\".\"},{\"type\":\"fraction\",\"value\":\"00\"},{\"type\":\"literal\",\"value\":\"\u{a0}\"},{\"type\":\"currency\",\"value\":\"\u{62c}.\u{645}.\"},{\"type\":\"literal\",\"value\":\"\u{200f}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('he', {style: 'unit', unit: 'day', unitDisplay: 'long'}).formatToParts(-2))",
            "[{\"type\":\"unit\",\"value\":\"\u{5d9}\u{5d5}\u{5de}\u{5d9}\u{5d9}\u{5dd}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'long', signDisplay: 'always'}).formatToParts(2))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\u{627}\u{646}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'long'}).formatToParts(-1))",
            "[{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\"}]",
        ),
        (
            "JSON.stringify(new Intl.NumberFormat('ar', {style: 'unit', unit: 'day', unitDisplay: 'long'}).formatToParts(-0))",
            "[{\"type\":\"literal\",\"value\":\"\u{200e}\"},{\"type\":\"minusSign\",\"value\":\"-\"},{\"type\":\"integer\",\"value\":\"0\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"unit\",\"value\":\"\u{64a}\u{648}\u{645}\"}]",
        ),
    ]);
}

#[test]
fn display_names_name_as_the_reference() {
    // The reference browser's (Chrome 156) answers; the generator compared 111,075 names of 75 locales.
    check(&[
        (
            "(() => { try { Intl.DisplayNames(); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Constructor Intl.DisplayNames requires 'new'",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {}); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', 5); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'x'}); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Value x out of range for Intl.DisplayNames options property type",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'region', style: 'x'}); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: Value x out of range for Intl.DisplayNames options property style",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'region'}).of(); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'region'}).of('xyz'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'language'}).of('en-u-ca-x'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'language'}).of('root'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'script'}).of('Lat'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'currency'}).of('US'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'calendar'}).of('gregorian'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'calendar'}).of('a'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'dateTimeField'}).of('years'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { Intl.DisplayNames.prototype.of.call({}, 'a'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "TypeError: Method Intl.DisplayNames.prototype.of called on incompatible receiver #<Object>",
        ),
        (
            "JSON.stringify(new Intl.DisplayNames('en', {type: 'region'}).resolvedOptions())",
            "{\"locale\":\"en\",\"style\":\"long\",\"type\":\"region\",\"fallback\":\"code\"}",
        ),
        (
            "JSON.stringify(new Intl.DisplayNames('de-AT-u-ca-buddhist', {type: 'language', style: 'short', fallback: 'none', languageDisplay: 'standard'}).resolvedOptions())",
            "{\"locale\":\"de-AT\",\"style\":\"short\",\"type\":\"language\",\"fallback\":\"none\",\"languageDisplay\":\"standard\"}",
        ),
        (
            "JSON.stringify(new Intl.DisplayNames('en', {type: 'currency'}).resolvedOptions())",
            "{\"locale\":\"en\",\"style\":\"long\",\"type\":\"currency\",\"fallback\":\"code\"}",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('ZZ'))",
            "Unknown Region",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', fallback: 'none'}).of('XQ'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('xq'))",
            "xq",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('419'))",
            "Latin America",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('en-us'))",
            "American English",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', languageDisplay: 'standard'}).of('en-US'))",
            "English (United States)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('zh-Hant-HK'))",
            "Chinese (Traditional, Hong Kong)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', languageDisplay: 'standard'}).of('zh-Hant-HK'))",
            "Chinese (Traditional, Hong Kong)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('sr-Latn-ME-variant'))",
            "Serbian (Latin, Montenegro, VARIANT)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', fallback: 'none'}).of('xyz'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('xyz'))",
            "xyz",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('xyz-Latn-US'))",
            "xyz (Latin, United States)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('EN-gb'))",
            "British English",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script'}).of('latn'))",
            "latn",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script', fallback: 'none'}).of('Zzzz'))",
            "Unknown Script",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script'}).of('Qaaa'))",
            "Qaaa",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency'}).of('usd'))",
            "US Dollar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency', style: 'short'}).of('usd'))",
            "US Dollar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency', style: 'narrow'}).of('usd'))",
            "US Dollar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency'}).of('XYZ'))",
            "XYZ",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency', fallback: 'none'}).of('XYZ'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar'}).of('gregory'))",
            "Gregorian Calendar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar'}).of('islamic-umalqura'))",
            "Hijri Calendar (Umm al-Qura)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar', style: 'short'}).of('gregory'))",
            "Gregorian Calendar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar'}).of('abc'))",
            "abc",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar', fallback: 'none'}).of('abc'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'calendar'}).of('Gregory'))",
            "Gregory",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'dateTimeField'}).of('dayPeriod'))",
            "AM/PM",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'dateTimeField', style: 'short'}).of('weekOfYear'))",
            "wk.",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'dateTimeField', style: 'narrow'}).of('timeZoneName'))",
            "zone",
        ),
        (
            "String(new Intl.DisplayNames('de', {type: 'dateTimeField'}).of('weekday'))",
            "Wochentag",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'short'}).of('en-US'))",
            "US English",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'narrow'}).of('en-US'))",
            "US English",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'short'}).of('US'))",
            "US",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'narrow'}).of('GB'))",
            "UK",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'short'}).of('HK'))",
            "Hong Kong",
        ),
        (
            "String(new Intl.DisplayNames('fr', {type: 'language'}).of('de-CH'))",
            "allemand suisse",
        ),
        (
            "String(new Intl.DisplayNames('ja', {type: 'language'}).of('en-Latn-US'))",
            "\u{82f1}\u{8a9e} (\u{30e9}\u{30c6}\u{30f3}\u{6587}\u{5b57}\u{3001}\u{30a2}\u{30e1}\u{30ea}\u{30ab}\u{5408}\u{8846}\u{56fd})",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('de-1996'))",
            "German (1996)",
        ),
        (
            "String(new Intl.DisplayNames('xx', {type: 'region'}).of('US'))",
            "United States",
        ),
        (
            "Intl.DisplayNames.length + ' ' + Intl.DisplayNames.prototype.of.length",
            "2 1",
        ),
        (
            "Object.getOwnPropertyNames(Intl.DisplayNames.prototype).join()",
            "constructor,resolvedOptions,of",
        ),
        (
            "JSON.stringify(Intl.DisplayNames.supportedLocalesOf(['en', 'xx']))",
            "[\"en\"]",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'language'}).of('en-'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "(() => { try { new Intl.DisplayNames('en', {type: 'currency'}).of('US$'); } catch (e) { return e.name + ': ' + e.message; } })()",
            "RangeError: invalid_argument",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('mo'))",
            "Romanian",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('in'))",
            "Indonesian",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('DD'))",
            "Germany",
        ),
    ]);
}

#[test]
fn display_names_compose_as_the_reference() {
    // ICU's composition, its cases and quirks, as the reference browser answers (Chrome 156).
    check(&[
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('us'))",
            "us",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('Us'))",
            "Us",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', fallback: 'none'}).of('us'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script'}).of('Latn'))",
            "Latin",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script'}).of('LATN'))",
            "LATN",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script', fallback: 'none'}).of('latn'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency'}).of('Usd'))",
            "US Dollar",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency', fallback: 'none'}).of('xyz'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'currency'}).of('xyz'))",
            "xyz",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('ZH'))",
            "Chinese",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', fallback: 'none'}).of('en-XQ'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', fallback: 'none'}).of('xyz-US'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('en-XQ'))",
            "English (XQ)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', fallback: 'none'}).of('en-Qaaa'))",
            "undefined",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('en-Qaaa'))",
            "English (Qaaa)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('und'))",
            "root",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('und-US'))",
            "root (United States)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('es-419'))",
            "Latin American Spanish",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', languageDisplay: 'standard'}).of('es-419'))",
            "Spanish (Latin America)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('zh-Hans'))",
            "Simplified Chinese",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('zh-Hans-CN'))",
            "Chinese (Simplified, China)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'short'}).of('zh-Hans'))",
            "Simplified Chinese",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('sr-Latn'))",
            "Serbian (Latin)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('nl-BE'))",
            "Flemish",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('ro-MD'))",
            "Moldavian",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('en-GB-oxendict'))",
            "British English (Oxford English Dictionary spelling)",
        ),
        (
            "String(new Intl.DisplayNames('de', {type: 'language'}).of('en-GB'))",
            "Englisch (Vereinigtes K\u{f6}nigreich)",
        ),
        (
            "String(new Intl.DisplayNames('de', {type: 'language'}).of('en-Latn-GB'))",
            "Englisch (Lateinisch, Vereinigtes K\u{f6}nigreich)",
        ),
        (
            "String(new Intl.DisplayNames('ar', {type: 'language'}).of('en-Latn-GB'))",
            "\u{627}\u{644}\u{625}\u{646}\u{62c}\u{644}\u{64a}\u{632}\u{64a}\u{629} (\u{627}\u{644}\u{644}\u{627}\u{62a}\u{64a}\u{646}\u{64a}\u{629}\u{60c} \u{627}\u{644}\u{645}\u{645}\u{644}\u{643}\u{629} \u{627}\u{644}\u{645}\u{62a}\u{62d}\u{62f}\u{629})",
        ),
        (
            "String(new Intl.DisplayNames('zh', {type: 'language'}).of('en-Latn-GB'))",
            "\u{82f1}\u{8bed}\u{ff08}\u{62c9}\u{4e01}\u{6587}\u{ff0c}\u{82f1}\u{56fd}\u{ff09}",
        ),
        (
            "String(new Intl.DisplayNames('ko', {type: 'language'}).of('en-Latn-GB'))",
            "\u{c601}\u{c5b4}(\u{b85c}\u{b9c8}\u{c790}, \u{c601}\u{ad6d})",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region'}).of('001'))",
            "001",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'short'}).of('PS'))",
            "Palestine",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'narrow'}).of('PS'))",
            "Palestine",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script', style: 'short'}).of('Hans'))",
            "Simplified",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script', style: 'narrow'}).of('Hans'))",
            "Simplified",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'script'}).of('Hans'))",
            "Simplified",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'region', style: 'short'}).of('BA'))",
            "Bosnia",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'short'}).of('az'))",
            "Azeri",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'narrow'}).of('en-GB'))",
            "UK English",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language', style: 'short', languageDisplay: 'standard'}).of('en-GB'))",
            "English (UK)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('de-CH-1901'))",
            "Swiss High German (1901)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('ca-ES-valencia'))",
            "Catalan (Spain, VALENCIA)",
        ),
        (
            "String(new Intl.DisplayNames('en', {type: 'language'}).of('sl-rozaj-biske'))",
            "Slovenian (BISKE_ROZAJ)",
        ),
    ]);
}

#[test]
fn display_names_in_other_languages_as_the_reference() {
    // Composed names, calendars, fields and currencies in other languages (Chrome 156), a sample.
    check(&[
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('af', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"Engels (Latyn, Verenigde Koninkryk)\",\"Chinees (Tradisioneel, Hongkong)\",\"Serwies (Cyrillies, Montenegro)\",\"xyz (Latyn, Verenigde State van Amerika)\",\"Duits (Switserland, 1901)\",\"Engels (Verenigde Koninkryk, OXENDICT)\",\"Spaans (Latyns-Amerika)\",\"Portugees (Brasili\u{eb})\",\"root (Verenigde State van Amerika)\",\"Moderne Standaardarabies\",\"Vlaams\",\"Frans (Kanada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ak', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"en (Latn, United Kingdom)\",\"zh (Hant, Hong Kong)\",\"sr (Cyrl, Montenegro)\",\"xyz (Latn, United States)\",\"de (Switzerland, 1901)\",\"en (United Kingdom, OXENDICT)\",\"es (Latin America)\",\"pt (Brazil)\",\"root (United States)\",\"ar (001)\",\"nl (Belgium)\",\"fr (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('am', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{12a5}\u{1295}\u{130d}\u{120a}\u{12dd}\u{129b} (\u{120b}\u{1272}\u{1295}\u{1363} \u{12e9}\u{12ac})\",\"\u{127b}\u{12ed}\u{1295}\u{129b} (\u{1263}\u{1205}\u{120b}\u{12ca}\u{1363} \u{1206}\u{1295}\u{130d} \u{12ae}\u{1295}\u{130d})\",\"\u{1230}\u{122d}\u{1265}\u{12eb}\u{129b} (\u{1232}\u{12ed}\u{122a}\u{120d}\u{12ad}\u{1363} \u{121e}\u{1295}\u{1270}\u{1294}\u{130d}\u{122e})\",\"xyz (\u{120b}\u{1272}\u{1295}\u{1363} \u{12e9}\u{1293}\u{12ed}\u{1275}\u{12f5} \u{1235}\u{1274}\u{1275}\u{1235})\",\"\u{12e8}\u{1235}\u{12ca}\u{12dd} \u{12a8}\u{134d}\u{1270}\u{129b} \u{1300}\u{122d}\u{1218}\u{1295}\u{129b} (1901)\",\"\u{12e8}\u{12e9}\u{1293}\u{12ed}\u{1275}\u{12f5} \u{12aa}\u{1295}\u{130d}\u{12f0}\u{121d} \u{12a5}\u{1295}\u{130d}\u{120a}\u{12dd}\u{129b} (OXENDICT)\",\"\u{12e8}\u{120b}\u{1272}\u{1295} \u{12a0}\u{121c}\u{122a}\u{12ab} \u{1235}\u{1353}\u{1292}\u{123d}\",\"\u{12e8}\u{1265}\u{122b}\u{12da}\u{120d} \u{1356}\u{122d}\u{1279}\u{130b}\u{120d}\u{129b}\",\"root (\u{12e9}\u{1293}\u{12ed}\u{1275}\u{12f5} \u{1235}\u{1274}\u{1275}\u{1235})\",\"\u{12d8}\u{1218}\u{1293}\u{12ca} \u{1218}\u{12f0}\u{1260}\u{129b} \u{12d3}\u{1228}\u{1265}\u{129b}\",\"\u{134d}\u{120c}\u{121a}\u{123d}\",\"\u{12e8}\u{12ab}\u{1293}\u{12f3} \u{1348}\u{1228}\u{1295}\u{1233}\u{12ed}\u{129b}\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ar', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{627}\u{644}\u{625}\u{646}\u{62c}\u{644}\u{64a}\u{632}\u{64a}\u{629} (\u{627}\u{644}\u{644}\u{627}\u{62a}\u{64a}\u{646}\u{64a}\u{629}\u{60c} \u{627}\u{644}\u{645}\u{645}\u{644}\u{643}\u{629} \u{627}\u{644}\u{645}\u{62a}\u{62d}\u{62f}\u{629})\",\"\u{627}\u{644}\u{635}\u{64a}\u{646}\u{64a}\u{629} (\u{627}\u{644}\u{62a}\u{642}\u{644}\u{64a}\u{62f}\u{64a}\u{629}\u{60c} \u{647}\u{648}\u{646}\u{63a} \u{643}\u{648}\u{646}\u{63a})\",\"\u{627}\u{644}\u{635}\u{631}\u{628}\u{64a}\u{629} (\u{627}\u{644}\u{633}\u{64a}\u{631}\u{64a}\u{644}\u{64a}\u{629}\u{60c} \u{627}\u{644}\u{62c}\u{628}\u{644} \u{627}\u{644}\u{623}\u{633}\u{648}\u{62f})\",\"xyz (\u{627}\u{644}\u{644}\u{627}\u{62a}\u{64a}\u{646}\u{64a}\u{629}\u{60c} \u{627}\u{644}\u{648}\u{644}\u{627}\u{64a}\u{627}\u{62a} \u{627}\u{644}\u{645}\u{62a}\u{62d}\u{62f}\u{629})\",\"\u{627}\u{644}\u{623}\u{644}\u{645}\u{627}\u{646}\u{64a}\u{629} (\u{633}\u{648}\u{64a}\u{633}\u{631}\u{627}\u{60c} 1901)\",\"\u{627}\u{644}\u{625}\u{646}\u{62c}\u{644}\u{64a}\u{632}\u{64a}\u{629} (\u{627}\u{644}\u{645}\u{645}\u{644}\u{643}\u{629} \u{627}\u{644}\u{645}\u{62a}\u{62d}\u{62f}\u{629}\u{60c} OXENDICT)\",\"\u{627}\u{644}\u{625}\u{633}\u{628}\u{627}\u{646}\u{64a}\u{629} (\u{623}\u{645}\u{631}\u{64a}\u{643}\u{627} \u{627}\u{644}\u{644}\u{627}\u{62a}\u{64a}\u{646}\u{64a}\u{629})\",\"\u{627}\u{644}\u{628}\u{631}\u{62a}\u{63a}\u{627}\u{644}\u{64a}\u{629} (\u{627}\u{644}\u{628}\u{631}\u{627}\u{632}\u{64a}\u{644})\",\"root (\u{627}\u{644}\u{648}\u{644}\u{627}\u{64a}\u{627}\u{62a} \u{627}\u{644}\u{645}\u{62a}\u{62d}\u{62f}\u{629})\",\"\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629} (001)\",\"\u{627}\u{644}\u{647}\u{648}\u{644}\u{646}\u{62f}\u{64a}\u{629} (\u{628}\u{644}\u{62c}\u{64a}\u{643}\u{627})\",\"\u{627}\u{644}\u{641}\u{631}\u{646}\u{633}\u{64a}\u{629} (\u{643}\u{646}\u{62f}\u{627})\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('az', {type: 'region'}).of(c)))",
            "[\"United States\",\"Germany\",\"us\",\"Latin America\",\"001\",\"Kosovo\",\"European Union\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('bg', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"\u{433}\u{440}\u{438}\u{433}\u{43e}\u{440}\u{438}\u{430}\u{43d}\u{441}\u{43a}\u{438} \u{43a}\u{430}\u{43b}\u{435}\u{43d}\u{434}\u{430}\u{440}\",\"\u{44f}\u{43f}\u{43e}\u{43d}\u{441}\u{43a}\u{438} \u{43a}\u{430}\u{43b}\u{435}\u{43d}\u{434}\u{430}\u{440}\",\"\u{438}\u{441}\u{43b}\u{44f}\u{43c}\u{441}\u{43a}\u{438} \u{43a}\u{430}\u{43b}\u{435}\u{43d}\u{434}\u{430}\u{440} \u{425}\u{438}\u{434}\u{436}\u{440}\u{430}\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('bn', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"\u{9af}\u{9c1}\u{997}\",\"\u{9b8}\u{9aa}\u{9cd}\u{9a4}\u{9be}\u{9b9}\",\"AM/PM\",\"\u{985}\u{99e}\u{9cd}\u{99a}\u{9b2}\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('bs', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"US Dollar\",\"Euro\",\"xyz\",\"Japanese Yen\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('cs', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"angli\u{10d}tina (latinka, Spojen\u{e9} kr\u{e1}lovstv\u{ed})\",\"\u{10d}\u{ed}n\u{161}tina (tradi\u{10d}n\u{ed}, Hongkong)\",\"srb\u{161}tina (cyrilice, \u{10c}ern\u{e1} Hora)\",\"xyz (latinka, Spojen\u{e9} st\u{e1}ty)\",\"n\u{11b}m\u{10d}ina standardn\u{ed} (\u{160}v\u{fd}carsko) (1901)\",\"angli\u{10d}tina (Velk\u{e1} Brit\u{e1}nie) (OXENDICT)\",\"\u{161}pan\u{11b}l\u{161}tina (Latinsk\u{e1} Amerika)\",\"portugal\u{161}tina (Braz\u{ed}lie)\",\"root (Spojen\u{e9} st\u{e1}ty)\",\"arab\u{161}tina (modern\u{ed} standardn\u{ed})\",\"vl\u{e1}m\u{161}tina\",\"francouz\u{161}tina (Kanada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('da', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"engelsk (latinsk, Storbritannien)\",\"kinesisk (traditionelt, Hongkong)\",\"serbisk (kyrillisk, Montenegro)\",\"xyz (latinsk, USA)\",\"tysk (Schweiz, 1901)\",\"engelsk (Storbritannien, OXENDICT)\",\"spansk (Latinamerika)\",\"portugisisk (Brasilien)\",\"root (USA)\",\"arabisk (001)\",\"nederlandsk (Belgien)\",\"fransk (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('de', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"Englisch (Lateinisch, UK)\",\"Chinesisch (Traditionell, Hongkong)\",\"Serbisch (Kyrillisch, Montenegro)\",\"xyz (Lateinisch, USA)\",\"Schweizer Hochdeutsch (1901)\",\"Englisch (GB) (OXENDICT)\",\"Spanisch (Lateinamerika)\",\"Portugiesisch (Brasilien)\",\"root (USA)\",\"Modernes Hocharabisch\",\"Fl\u{e4}misch\",\"Franz\u{f6}sisch (Kanada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('el', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{391}\u{3b3}\u{3b3}\u{3bb}\u{3b9}\u{3ba}\u{3ac} (\u{39b}\u{3b1}\u{3c4}\u{3b9}\u{3bd}\u{3b9}\u{3ba}\u{3cc}, \u{397}\u{392})\",\"\u{39a}\u{3b9}\u{3bd}\u{3b5}\u{3b6}\u{3b9}\u{3ba}\u{3ac} (\u{3a0}\u{3b1}\u{3c1}\u{3b1}\u{3b4}\u{3bf}\u{3c3}\u{3b9}\u{3b1}\u{3ba}\u{3cc}, \u{3a7}\u{3bf}\u{3bd}\u{3b3}\u{3ba} \u{39a}\u{3bf}\u{3bd}\u{3b3}\u{3ba})\",\"\u{3a3}\u{3b5}\u{3c1}\u{3b2}\u{3b9}\u{3ba}\u{3ac} (\u{39a}\u{3c5}\u{3c1}\u{3b9}\u{3bb}\u{3bb}\u{3b9}\u{3ba}\u{3cc}, \u{39c}\u{3b1}\u{3c5}\u{3c1}\u{3bf}\u{3b2}\u{3bf}\u{3cd}\u{3bd}\u{3b9}\u{3bf})\",\"xyz (\u{39b}\u{3b1}\u{3c4}\u{3b9}\u{3bd}\u{3b9}\u{3ba}\u{3cc}, \u{397}\u{3a0}\u{391})\",\"\u{393}\u{3b5}\u{3c1}\u{3bc}\u{3b1}\u{3bd}\u{3b9}\u{3ba}\u{3ac} (\u{395}\u{3bb}\u{3b2}\u{3b5}\u{3c4}\u{3af}\u{3b1}, 1901)\",\"\u{391}\u{3b3}\u{3b3}\u{3bb}\u{3b9}\u{3ba}\u{3ac} (\u{397}\u{392}, OXENDICT)\",\"\u{399}\u{3c3}\u{3c0}\u{3b1}\u{3bd}\u{3b9}\u{3ba}\u{3ac} (\u{39b}\u{3b1}\u{3c4}\u{3b9}\u{3bd}\u{3b9}\u{3ba}\u{3ae} \u{391}\u{3bc}\u{3b5}\u{3c1}\u{3b9}\u{3ba}\u{3ae})\",\"\u{3a0}\u{3bf}\u{3c1}\u{3c4}\u{3bf}\u{3b3}\u{3b1}\u{3bb}\u{3b9}\u{3ba}\u{3ac} (\u{392}\u{3c1}\u{3b1}\u{3b6}\u{3b9}\u{3bb}\u{3af}\u{3b1})\",\"root (\u{397}\u{3a0}\u{391})\",\"\u{391}\u{3c1}\u{3b1}\u{3b2}\u{3b9}\u{3ba}\u{3ac} (001)\",\"\u{39f}\u{3bb}\u{3bb}\u{3b1}\u{3bd}\u{3b4}\u{3b9}\u{3ba}\u{3ac} (\u{392}\u{3ad}\u{3bb}\u{3b3}\u{3b9}\u{3bf})\",\"\u{393}\u{3b1}\u{3bb}\u{3bb}\u{3b9}\u{3ba}\u{3ac} (\u{39a}\u{3b1}\u{3bd}\u{3b1}\u{3b4}\u{3ac}\u{3c2})\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('en', {type: 'region'}).of(c)))",
            "[\"United States\",\"Germany\",\"us\",\"Latin America\",\"001\",\"Kosovo\",\"European Union\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('es', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"calendario gregoriano\",\"calendario japon\u{e9}s\",\"calendario hijri tabular\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('et', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"ajastu\",\"n\u{e4}d\",\"enne/p\u{e4}rast l\u{f5}unat\",\"v\u{f6}\u{f6}nd\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('fa', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"\u{62f}\u{644}\u{627}\u{631} \u{622}\u{645}\u{631}\u{6cc}\u{6a9}\u{627}\",\"\u{6cc}\u{648}\u{631}\u{648}\",\"xyz\",\"\u{6cc}\u{646} \u{698}\u{627}\u{67e}\u{646}\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('fil', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"Ingles (Latin, United Kingdom)\",\"Chinese (Tradisyonal, Hong Kong)\",\"Serbian (Cyrillic, Montenegro)\",\"xyz (Latin, Estados Unidos)\",\"German (Switzerland, 1901)\",\"Ingles (United Kingdom, OXENDICT)\",\"Latin American na Espanyol\",\"Portuguese (Brazil)\",\"root (Estados Unidos)\",\"Modern Standard Arabic\",\"Flemish\",\"French (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('fr', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"anglais (latin, Royaume-Uni)\",\"chinois (traditionnel, Hong Kong)\",\"serbe (cyrillique, Mont\u{e9}n\u{e9}gro)\",\"xyz (latin, \u{c9}tats-Unis)\",\"allemand (Suisse, 1901)\",\"anglais (Royaume-Uni, OXENDICT)\",\"espagnol (Am\u{e9}rique latine)\",\"portugais (Br\u{e9}sil)\",\"root (\u{c9}tats-Unis)\",\"arabe (001)\",\"n\u{e9}erlandais (Belgique)\",\"fran\u{e7}ais (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('gu', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{a85}\u{a82}\u{a97}\u{acd}\u{ab0}\u{ac7}\u{a9c}\u{ac0} (\u{ab2}\u{ac7}\u{a9f}\u{abf}\u{aa8}, \u{aaf}\u{ac1}.\u{a95}\u{ac7}.)\",\"\u{a9a}\u{abe}\u{a87}\u{aa8}\u{ac0}\u{a9d} (\u{aaa}\u{ab0}\u{a82}\u{aaa}\u{ab0}\u{abe}\u{a97}\u{aa4}, \u{ab9}\u{acb}\u{a82}\u{a97}\u{a95}\u{acb}\u{a82}\u{a97})\",\"\u{ab8}\u{ab0}\u{acd}\u{aac}\u{abf}\u{aaf}\u{aa8} (\u{ab8}\u{abf}\u{ab0}\u{abf}\u{ab2}\u{abf}\u{a95}, \u{aae}\u{ac9}\u{aa8}\u{acd}\u{a9f}\u{ac7}\u{aa8}\u{ac7}\u{a97}\u{acd}\u{ab0}\u{acb})\",\"xyz (\u{ab2}\u{ac7}\u{a9f}\u{abf}\u{aa8}, \u{aaf}\u{ac1}.\u{a8f}\u{ab8}.)\",\"\u{ab8}\u{acd}\u{ab5}\u{abf}\u{ab8} \u{ab9}\u{abe}\u{aaf} \u{a9c}\u{ab0}\u{acd}\u{aae}\u{aa8} (1901)\",\"\u{aaf}\u{ac1}.\u{a95}\u{ac7}. \u{a85}\u{a82}\u{a97}\u{acd}\u{ab0}\u{ac7}\u{a9c}\u{ac0} (OXENDICT)\",\"\u{ab2}\u{ac7}\u{a9f}\u{abf}\u{aa8} \u{a85}\u{aae}\u{ac7}\u{ab0}\u{abf}\u{a95}\u{aa8} \u{ab8}\u{acd}\u{aaa}\u{ac7}\u{aa8}\u{abf}\u{ab6}\",\"\u{aac}\u{acd}\u{ab0}\u{abe}\u{a9d}\u{abf}\u{ab2}\u{ac0}\u{aaf}\u{aa8} \u{aaa}\u{acb}\u{ab0}\u{acd}\u{a9f}\u{ac1}\u{a97}\u{ac0}\u{a9d}\",\"root (\u{aaf}\u{ac1}.\u{a8f}\u{ab8}.)\",\"\u{aae}\u{acb}\u{aa1}\u{ab0}\u{acd}\u{aa8} \u{ab8}\u{acd}\u{a9f}\u{abe}\u{aa8}\u{acd}\u{aa1}\u{ab0}\u{acd}\u{aa1} \u{a85}\u{ab0}\u{aac}\u{ac0}\",\"\u{aab}\u{acd}\u{ab2}\u{ac7}\u{aae}\u{abf}\u{ab6}\",\"\u{aab}\u{acd}\u{ab0}\u{ac7}\u{aa8}\u{acd}\u{a9a} (\u{a95}\u{ac7}\u{aa8}\u{ac7}\u{aa1}\u{abe})\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('he', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{5d0}\u{5e0}\u{5d2}\u{5dc}\u{5d9}\u{5ea} (\u{5dc}\u{5d8}\u{5d9}\u{5e0}\u{5d9}, \u{5d1}\u{5e8}\u{5d9}\u{5d8}\u{5e0}\u{5d9}\u{5d4})\",\"\u{5e1}\u{5d9}\u{5e0}\u{5d9}\u{5ea} (\u{5de}\u{5e1}\u{5d5}\u{5e8}\u{5ea}\u{5d9}, \u{5d4}\u{5d5}\u{5e0}\u{5d2} \u{5e7}\u{5d5}\u{5e0}\u{5d2})\",\"\u{5e1}\u{5e8}\u{5d1}\u{5d9}\u{5ea} (\u{5e7}\u{5d9}\u{5e8}\u{5d9}\u{5dc}\u{5d9}, \u{5de}\u{5d5}\u{5e0}\u{5d8}\u{5e0}\u{5d2}\u{5e8}\u{5d5})\",\"xyz (\u{5dc}\u{5d8}\u{5d9}\u{5e0}\u{5d9}, \u{5d0}\u{5e8}\u{5d4}\u{5f4}\u{5d1})\",\"\u{5d2}\u{5e8}\u{5de}\u{5e0}\u{5d9}\u{5ea} (\u{5e9}\u{5d5}\u{5d5}\u{5d9}\u{5d9}\u{5e5}, 1901)\",\"\u{5d0}\u{5e0}\u{5d2}\u{5dc}\u{5d9}\u{5ea} (\u{5d1}\u{5e8}\u{5d9}\u{5d8}\u{5e0}\u{5d9}\u{5d4}, \u{5d0}\u{5d9}\u{5d5}\u{5ea} \u{5de}\u{5d9}\u{5dc}\u{5d5}\u{5df} \u{5d0}\u{5e0}\u{5d2}\u{5dc}\u{5d9}\u{5ea} \u{5e9}\u{5dc} \u{5d0}\u{5d5}\u{5e7}\u{5e1}\u{5e4}\u{5d5}\u{5e8}\u{5d3})\",\"\u{5e1}\u{5e4}\u{5e8}\u{5d3}\u{5d9}\u{5ea} (\u{5d0}\u{5de}\u{5e8}\u{5d9}\u{5e7}\u{5d4} \u{5d4}\u{5dc}\u{5d8}\u{5d9}\u{5e0}\u{5d9}\u{5ea})\",\"\u{5e4}\u{5d5}\u{5e8}\u{5d8}\u{5d5}\u{5d2}\u{5d6}\u{5d9}\u{5ea} (\u{5d1}\u{5e8}\u{5d6}\u{5d9}\u{5dc})\",\"root (\u{5d0}\u{5e8}\u{5d4}\u{5f4}\u{5d1})\",\"\u{5e2}\u{5e8}\u{5d1}\u{5d9}\u{5ea} (001)\",\"\u{5d4}\u{5d5}\u{5dc}\u{5e0}\u{5d3}\u{5d9}\u{5ea} (\u{5d1}\u{5dc}\u{5d2}\u{5d9}\u{5d4})\",\"\u{5e6}\u{5e8}\u{5e4}\u{5ea}\u{5d9}\u{5ea} (\u{5e7}\u{5e0}\u{5d3}\u{5d4})\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('hi', {type: 'region'}).of(c)))",
            "[\"\u{938}\u{902}\u{92f}\u{941}\u{915}\u{94d}\u{924} \u{930}\u{93e}\u{91c}\u{94d}\u{92f}\",\"\u{91c}\u{930}\u{94d}\u{92e}\u{928}\u{940}\",\"us\",\"\u{932}\u{948}\u{91f}\u{93f}\u{928} \u{905}\u{92e}\u{947}\u{930}\u{93f}\u{915}\u{93e}\",\"001\",\"\u{915}\u{94b}\u{938}\u{94b}\u{935}\u{94b}\",\"\u{92f}\u{942}\u{930}\u{94b}\u{92a}\u{940}\u{92f} \u{938}\u{902}\u{918}\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('hr', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"gregorijanski kalendar\",\"japanski kalendar\",\"hijri kalendar (tabularni, civilna epoha)\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('hu', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"\u{e9}ra\",\"h\u{e9}t\",\"napszak\",\"id\u{151}z\u{f3}na\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('id', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"Dolar Amerika Serikat\",\"Euro\",\"xyz\",\"Yen Jepang\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ja', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{82f1}\u{8a9e} (\u{30e9}\u{30c6}\u{30f3}\u{6587}\u{5b57}\u{3001}\u{30a4}\u{30ae}\u{30ea}\u{30b9})\",\"\u{4e2d}\u{56fd}\u{8a9e} (\u{7e41}\u{4f53}\u{5b57}\u{3001}\u{9999}\u{6e2f})\",\"\u{30bb}\u{30eb}\u{30d3}\u{30a2}\u{8a9e} (\u{30ad}\u{30ea}\u{30eb}\u{6587}\u{5b57}\u{3001}\u{30e2}\u{30f3}\u{30c6}\u{30cd}\u{30b0}\u{30ed})\",\"xyz (\u{30e9}\u{30c6}\u{30f3}\u{6587}\u{5b57}\u{3001}\u{30a2}\u{30e1}\u{30ea}\u{30ab}\u{5408}\u{8846}\u{56fd})\",\"\u{6a19}\u{6e96}\u{30c9}\u{30a4}\u{30c4}\u{8a9e} (\u{30b9}\u{30a4}\u{30b9}) (1901)\",\"\u{30a4}\u{30ae}\u{30ea}\u{30b9}\u{82f1}\u{8a9e} (OXENDICT)\",\"\u{30b9}\u{30da}\u{30a4}\u{30f3}\u{8a9e} (\u{30e9}\u{30c6}\u{30f3}\u{30a2}\u{30e1}\u{30ea}\u{30ab})\",\"\u{30dd}\u{30eb}\u{30c8}\u{30ac}\u{30eb}\u{8a9e} (\u{30d6}\u{30e9}\u{30b8}\u{30eb})\",\"root (\u{30a2}\u{30e1}\u{30ea}\u{30ab}\u{5408}\u{8846}\u{56fd})\",\"\u{73fe}\u{4ee3}\u{6a19}\u{6e96}\u{30a2}\u{30e9}\u{30d3}\u{30a2}\u{8a9e}\",\"\u{30d5}\u{30e9}\u{30de}\u{30f3}\u{8a9e}\",\"\u{30d5}\u{30e9}\u{30f3}\u{30b9}\u{8a9e} (\u{30ab}\u{30ca}\u{30c0})\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('kk', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"en (Latn, United Kingdom)\",\"zh (Hant, Hong Kong)\",\"sr (Cyrl, Montenegro)\",\"xyz (Latn, United States)\",\"de (Switzerland, 1901)\",\"en (United Kingdom, OXENDICT)\",\"es (Latin America)\",\"pt (Brazil)\",\"root (United States)\",\"ar (001)\",\"nl (Belgium)\",\"fr (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('kn', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{c87}\u{c82}\u{c97}\u{ccd}\u{cb2}\u{cbf}\u{cb7}\u{ccd} (\u{cb2}\u{ccd}\u{caf}\u{cbe}\u{c9f}\u{cbf}\u{ca8}\u{ccd}, \u{caf}\u{cc1}.\u{c95}\u{cc6}.)\",\"\u{c9a}\u{cc8}\u{ca8}\u{cc0}\u{cb8}\u{ccd} (\u{cb8}\u{cbe}\u{c82}\u{caa}\u{ccd}\u{cb0}\u{ca6}\u{cbe}\u{caf}\u{cbf}\u{c95}, \u{cb9}\u{cbe}\u{c82}\u{c97}\u{ccd} \u{c95}\u{cbe}\u{c82}\u{c97}\u{ccd})\",\"\u{cb8}\u{cc6}\u{cb0}\u{ccd}\u{cac}\u{cbf}\u{caf}\u{ca8}\u{ccd} (\u{cb8}\u{cbf}\u{cb0}\u{cbf}\u{cb2}\u{cbf}\u{c95}\u{ccd}, \u{cae}\u{cca}\u{c82}\u{c9f}\u{cc6}\u{ca8}\u{cc6}\u{c97}\u{ccd}\u{cb0}\u{ccb})\",\"xyz (\u{cb2}\u{ccd}\u{caf}\u{cbe}\u{c9f}\u{cbf}\u{ca8}\u{ccd}, \u{caf}\u{cc1}.\u{c8e}\u{cb8}\u{ccd})\",\"\u{cb8}\u{ccd}\u{cb5}\u{cbf}\u{cb8}\u{ccd} \u{cb9}\u{cc8} \u{c9c}\u{cb0}\u{ccd}\u{cae}\u{ca8}\u{ccd} (1901)\",\"\u{caf}\u{cc1}.\u{c95}\u{cc6}. \u{c87}\u{c82}\u{c97}\u{ccd}\u{cb2}\u{cbf}\u{cb7}\u{ccd} (OXENDICT)\",\"\u{cb2}\u{ccd}\u{caf}\u{cbe}\u{c9f}\u{cbf}\u{ca8}\u{ccd} \u{c85}\u{cae}\u{cc7}\u{cb0}\u{cbf}\u{c95}\u{ca8}\u{ccd} \u{cb8}\u{ccd}\u{caa}\u{ccd}\u{caf}\u{cbe}\u{ca8}\u{cbf}\u{cb7}\u{ccd}\",\"\u{cac}\u{ccd}\u{cb0}\u{cc6}\u{c9c}\u{cbf}\u{cb2}\u{cbf}\u{caf}\u{ca8}\u{ccd} \u{caa}\u{ccb}\u{cb0}\u{ccd}\u{c9a}\u{cc1}\u{c97}\u{cc0}\u{cb8}\u{ccd}\",\"root (\u{caf}\u{cc1}.\u{c8e}\u{cb8}\u{ccd})\",\"\u{c86}\u{ca7}\u{cc1}\u{ca8}\u{cbf}\u{c95} \u{caa}\u{ccd}\u{cb0}\u{cae}\u{cbe}\u{ca3}\u{cbf}\u{ca4} \u{c85}\u{cb0}\u{cc7}\u{cac}\u{cbf}\u{c95}\u{ccd}\",\"\u{cab}\u{ccd}\u{cb2}\u{cc6}\u{cae}\u{cbf}\u{cb7}\u{ccd}\",\"\u{c95}\u{cc6}\u{ca8}\u{cc6}\u{ca1}\u{cbf}\u{caf}\u{ca8}\u{ccd} \u{cab}\u{ccd}\u{cb0}\u{cc6}\u{c82}\u{c9a}\u{ccd}\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ko', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{c601}\u{c5b4}(\u{b85c}\u{b9c8}\u{c790}, \u{c601}\u{ad6d})\",\"\u{c911}\u{ad6d}\u{c5b4}(\u{bc88}\u{ccb4}, \u{d64d}\u{cf69})\",\"\u{c138}\u{b974}\u{be44}\u{c544}\u{c5b4}(\u{d0a4}\u{b9b4} \u{bb38}\u{c790}, \u{baac}\u{d14c}\u{b124}\u{adf8}\u{b85c})\",\"xyz(\u{b85c}\u{b9c8}\u{c790}, \u{bbf8}\u{ad6d})\",\"\u{b3c5}\u{c77c}\u{c5b4}(\u{c2a4}\u{c704}\u{c2a4}, 1901)\",\"\u{c601}\u{c5b4}(\u{c601}\u{ad6d}, OXENDICT)\",\"\u{c2a4}\u{d398}\u{c778}\u{c5b4}(\u{b77c}\u{d2f4} \u{c544}\u{ba54}\u{b9ac}\u{ce74})\",\"\u{d3ec}\u{b974}\u{d22c}\u{ac08}\u{c5b4}(\u{be0c}\u{b77c}\u{c9c8})\",\"root(\u{bbf8}\u{ad6d})\",\"\u{c544}\u{b78d}\u{c5b4}(001)\",\"\u{b124}\u{b35c}\u{b780}\u{b4dc}\u{c5b4}(\u{bca8}\u{ae30}\u{c5d0})\",\"\u{d504}\u{b791}\u{c2a4}\u{c5b4}(\u{ce90}\u{b098}\u{b2e4})\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('kok', {type: 'region'}).of(c)))",
            "[\"United States\",\"Germany\",\"us\",\"Latin America\",\"001\",\"Kosovo\",\"European Union\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('lt', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"Grigaliaus kalendorius\",\"japon\u{173} kalendorius\",\"musulmon\u{173} kalendorius (lentelinis, pilietin\u{117} era)\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('lv', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"\u{113}ra\",\"ned.\",\"priek\u{161}pusd./p\u{113}cpusd.\",\"laika josla\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('ml', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"\u{d2f}\u{d41}.\u{d0e}\u{d38}\u{d4d}. \u{d21}\u{d4b}\u{d33}\u{d7c}\",\"\u{d2f}\u{d42}\u{d31}\u{d4b}\",\"xyz\",\"\u{d1c}\u{d3e}\u{d2a}\u{d4d}\u{d2a}\u{d28}\u{d40}\u{d38}\u{d4d} \u{d2f}\u{d46}\u{d7b}\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ms', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"Inggeris (Latin, United Kingdom)\",\"Cina (Tradisional, Hong Kong)\",\"Serbia (Cyril, Montenegro)\",\"xyz (Latin, Amerika Syarikat)\",\"Jerman Halus Switzerland (1901)\",\"Inggeris British (Ejaan Kamus Inggeris Oxford)\",\"Sepanyol Amerika Latin\",\"Portugis Brazil\",\"root (Amerika Syarikat)\",\"Arab Standard Moden\",\"Flemish\",\"Perancis Kanada\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('nb', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"engelsk (latinsk, Storbritannia)\",\"kinesisk (tradisjonell, Hongkong)\",\"serbisk (kyrillisk, Montenegro)\",\"xyz (latinsk, USA)\",\"tysk (Sveits, 1901)\",\"engelsk (Storbritannia, OXENDICT)\",\"spansk (Latin-Amerika)\",\"portugisisk (Brasil)\",\"root (USA)\",\"arabisk (001)\",\"nederlandsk (Belgia)\",\"fransk (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('nl', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"Engels (Latijns, VK)\",\"Chinees (traditioneel, Hongkong)\",\"Servisch (Cyrillisch, Montenegro)\",\"xyz (Latijns, VS)\",\"Duits (Zwitserland, 1901)\",\"Engels (VK, Spelling volgens het Oxford English Dictionary)\",\"Spaans (Latijns-Amerika)\",\"Portugees (Brazili\u{eb})\",\"root (VS)\",\"modern standaard Arabisch\",\"Vlaams\",\"Frans (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('no', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"engelsk (latinsk, Storbritannia)\",\"kinesisk (tradisjonell, Hongkong)\",\"serbisk (kyrillisk, Montenegro)\",\"xyz (latinsk, USA)\",\"tysk (Sveits, 1901)\",\"engelsk (Storbritannia, OXENDICT)\",\"spansk (Latin-Amerika)\",\"portugisisk (Brasil)\",\"root (USA)\",\"arabisk (001)\",\"nederlandsk (Belgia)\",\"fransk (Canada)\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('pa', {type: 'region'}).of(c)))",
            "[\"United States\",\"Germany\",\"us\",\"Latin America\",\"001\",\"Kosovo\",\"European Union\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('pl', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"kalendarz gregoria\u{144}ski\",\"kalendarz japo\u{144}ski\",\"kalendarz muzu\u{142}ma\u{144}ski (metoda obliczeniowa)\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('pt', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"era\",\"sem.\",\"AM/PM\",\"fuso\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('ro', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"dolar american\",\"euro\",\"xyz\",\"yen japonez\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('sd', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"en (Latn, United Kingdom)\",\"zh (Hant, Hong Kong)\",\"sr (Cyrl, Montenegro)\",\"xyz (Latn, United States)\",\"de (Switzerland, 1901)\",\"en (United Kingdom, OXENDICT)\",\"es (Latin America)\",\"pt (Brazil)\",\"root (United States)\",\"ar (001)\",\"nl (Belgium)\",\"fr (Canada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('sk', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"angli\u{10d}tina (latinka, Spojen\u{e9} kr\u{e1}\u{13e}ovstvo)\",\"\u{10d}\u{ed}n\u{161}tina (tradi\u{10d}n\u{e9}, Hongkong)\",\"srb\u{10d}ina (cyrilika, \u{10c}ierna Hora)\",\"xyz (latinka, Spojen\u{e9} \u{161}t\u{e1}ty)\",\"nem\u{10d}ina (\u{160}vaj\u{10d}iarsko, 1901)\",\"angli\u{10d}tina (Spojen\u{e9} kr\u{e1}\u{13e}ovstvo, OXENDICT)\",\"\u{161}paniel\u{10d}ina (Latinsk\u{e1} Amerika)\",\"portugal\u{10d}ina (Braz\u{ed}lia)\",\"root (Spojen\u{e9} \u{161}t\u{e1}ty)\",\"arab\u{10d}ina (001)\",\"holand\u{10d}ina (Belgicko)\",\"franc\u{fa}z\u{161}tina (Kanada)\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('sl', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"angle\u{161}\u{10d}ina (latinica, ZK)\",\"kitaj\u{161}\u{10d}ina (tradicionalna pisava, Hongkong)\",\"srb\u{161}\u{10d}ina (cirilica, \u{10c}rna gora)\",\"xyz (latinica, ZDA)\",\"visoka nem\u{161}\u{10d}ina (\u{160}vica) (1901)\",\"angle\u{161}\u{10d}ina (ZK) (OXENDICT)\",\"latinskoameri\u{161}ka \u{161}pan\u{161}\u{10d}ina\",\"brazilska portugal\u{161}\u{10d}ina\",\"root (ZDA)\",\"sodobna standardna arab\u{161}\u{10d}ina\",\"flam\u{161}\u{10d}ina\",\"kanadska franco\u{161}\u{10d}ina\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('sr', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{435}\u{43d}\u{433}\u{43b}\u{435}\u{441}\u{43a}\u{438} (\u{43b}\u{430}\u{442}\u{438}\u{43d}\u{438}\u{446}\u{430}, \u{423}\u{41a})\",\"\u{43a}\u{438}\u{43d}\u{435}\u{441}\u{43a}\u{438} (\u{442}\u{440}\u{430}\u{434}\u{438}\u{446}\u{438}\u{43e}\u{43d}\u{430}\u{43b}\u{43d}\u{43e} \u{43a}\u{438}\u{43d}\u{435}\u{441}\u{43a}\u{43e} \u{43f}\u{438}\u{441}\u{43c}\u{43e}, \u{425}\u{43e}\u{43d}\u{433}\u{43a}\u{43e}\u{43d}\u{433})\",\"\u{441}\u{440}\u{43f}\u{441}\u{43a}\u{438} (\u{45b}\u{438}\u{440}\u{438}\u{43b}\u{438}\u{446}\u{430}, \u{426}\u{440}\u{43d}\u{430} \u{413}\u{43e}\u{440}\u{430})\",\"xyz (\u{43b}\u{430}\u{442}\u{438}\u{43d}\u{438}\u{446}\u{430}, \u{421}\u{410}\u{414})\",\"\u{43d}\u{435}\u{43c}\u{430}\u{447}\u{43a}\u{438} (\u{428}\u{432}\u{430}\u{458}\u{446}\u{430}\u{440}\u{441}\u{43a}\u{430}, 1901)\",\"\u{435}\u{43d}\u{433}\u{43b}\u{435}\u{441}\u{43a}\u{438} (\u{423}\u{41a}, OXENDICT)\",\"\u{448}\u{43f}\u{430}\u{43d}\u{441}\u{43a}\u{438} (\u{41b}\u{430}\u{442}\u{438}\u{43d}\u{441}\u{43a}\u{430} \u{410}\u{43c}\u{435}\u{440}\u{438}\u{43a}\u{430})\",\"\u{43f}\u{43e}\u{440}\u{442}\u{443}\u{433}\u{430}\u{43b}\u{441}\u{43a}\u{438} (\u{411}\u{440}\u{430}\u{437}\u{438}\u{43b})\",\"root (\u{421}\u{410}\u{414})\",\"\u{430}\u{440}\u{430}\u{43f}\u{441}\u{43a}\u{438} (001)\",\"\u{445}\u{43e}\u{43b}\u{430}\u{43d}\u{434}\u{441}\u{43a}\u{438} (\u{411}\u{435}\u{43b}\u{433}\u{438}\u{458}\u{430})\",\"\u{444}\u{440}\u{430}\u{43d}\u{446}\u{443}\u{441}\u{43a}\u{438} (\u{41a}\u{430}\u{43d}\u{430}\u{434}\u{430})\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('sv', {type: 'region'}).of(c)))",
            "[\"USA\",\"Tyskland\",\"us\",\"Latinamerika\",\"001\",\"Kosovo\",\"Europeiska unionen\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('sw', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"Kalenda ya Kigregori\",\"Kalenda ya Kijapani\",\"Kalenda ya Hijra (inayoanza usiku wa manane)\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('ta', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"\u{b95}\u{bbe}\u{bb2}\u{bae}\u{bcd}\",\"\u{bb5}\u{bbe}.\",\"AM/PM\",\"\u{bae}\u{ba3}\u{bcd}\u{b9f}\u{bb2}\u{bae}\u{bcd}\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('te', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"\u{c05}\u{c2e}\u{c46}\u{c30}\u{c3f}\u{c15}\u{c3e} \u{c21}\u{c3e}\u{c32}\u{c30}\u{c4d}\",\"\u{c2f}\u{c42}\u{c30}\u{c4b}\",\"xyz\",\"\u{c1c}\u{c2a}\u{c28}\u{c40}\u{c38}\u{c4d} \u{c2f}\u{c46}\u{c28}\u{c4d}\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('tr', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{130}ngilizce (Latin, Birle\u{15f}ik Krall\u{131}k)\",\"\u{c7}ince (Geleneksel, Hong Kong)\",\"S\u{131}rp\u{e7}a (Kiril, Karada\u{11f})\",\"xyz (Latin, Amerika Birle\u{15f}ik Devletleri)\",\"\u{130}svi\u{e7}re Y\u{fc}ksek Almancas\u{131} (1901)\",\"\u{130}ngiliz \u{130}ngilizcesi (OXENDICT)\",\"Latin Amerika \u{130}spanyolcas\u{131}\",\"Brezilya Portekizcesi\",\"root (Amerika Birle\u{15f}ik Devletleri)\",\"Modern Standart Arap\u{e7}a\",\"Flamanca\",\"Kanada Frans\u{131}zcas\u{131}\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('uk', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"\u{430}\u{43d}\u{433}\u{43b}\u{456}\u{439}\u{441}\u{44c}\u{43a}\u{430} (\u{43b}\u{430}\u{442}\u{438}\u{43d}\u{438}\u{446}\u{44f}, \u{412}\u{435}\u{43b}\u{438}\u{43a}\u{430} \u{411}\u{440}\u{438}\u{442}\u{430}\u{43d}\u{456}\u{44f})\",\"\u{43a}\u{438}\u{442}\u{430}\u{439}\u{441}\u{44c}\u{43a}\u{430} (\u{442}\u{440}\u{430}\u{434}\u{438}\u{446}\u{456}\u{439}\u{43d}\u{430}, \u{413}\u{43e}\u{43d}\u{43a}\u{43e}\u{43d}\u{433})\",\"\u{441}\u{435}\u{440}\u{431}\u{441}\u{44c}\u{43a}\u{430} (\u{43a}\u{438}\u{440}\u{438}\u{43b}\u{438}\u{446}\u{44f}, \u{427}\u{43e}\u{440}\u{43d}\u{43e}\u{433}\u{43e}\u{440}\u{456}\u{44f})\",\"xyz (\u{43b}\u{430}\u{442}\u{438}\u{43d}\u{438}\u{446}\u{44f}, \u{421}\u{43f}\u{43e}\u{43b}\u{443}\u{447}\u{435}\u{43d}\u{456} \u{428}\u{442}\u{430}\u{442}\u{438})\",\"\u{43d}\u{456}\u{43c}\u{435}\u{446}\u{44c}\u{43a}\u{430} (\u{428}\u{432}\u{435}\u{439}\u{446}\u{430}\u{440}\u{456}\u{44f}, 1901)\",\"\u{430}\u{43d}\u{433}\u{43b}\u{456}\u{439}\u{441}\u{44c}\u{43a}\u{430} (\u{412}\u{435}\u{43b}\u{438}\u{43a}\u{430} \u{411}\u{440}\u{438}\u{442}\u{430}\u{43d}\u{456}\u{44f}, OXENDICT)\",\"\u{456}\u{441}\u{43f}\u{430}\u{43d}\u{441}\u{44c}\u{43a}\u{430} (\u{41b}\u{430}\u{442}\u{438}\u{43d}\u{441}\u{44c}\u{43a}\u{430} \u{410}\u{43c}\u{435}\u{440}\u{438}\u{43a}\u{430})\",\"\u{43f}\u{43e}\u{440}\u{442}\u{443}\u{433}\u{430}\u{43b}\u{44c}\u{441}\u{44c}\u{43a}\u{430} (\u{411}\u{440}\u{430}\u{437}\u{438}\u{43b}\u{456}\u{44f})\",\"root (\u{421}\u{43f}\u{43e}\u{43b}\u{443}\u{447}\u{435}\u{43d}\u{456} \u{428}\u{442}\u{430}\u{442}\u{438})\",\"\u{430}\u{440}\u{430}\u{431}\u{441}\u{44c}\u{43a}\u{430} (001)\",\"\u{43d}\u{456}\u{434}\u{435}\u{440}\u{43b}\u{430}\u{43d}\u{434}\u{441}\u{44c}\u{43a}\u{430} (\u{411}\u{435}\u{43b}\u{44c}\u{433}\u{456}\u{44f})\",\"\u{444}\u{440}\u{430}\u{43d}\u{446}\u{443}\u{437}\u{44c}\u{43a}\u{430} (\u{41a}\u{430}\u{43d}\u{430}\u{434}\u{430})\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('ur', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{627}\u{646}\u{6af}\u{631}\u{6cc}\u{632}\u{6cc} (\u{644}\u{627}\u{637}\u{6cc}\u{646}\u{6cc}\u{60c}\u{6cc}\u{648} \u{6a9}\u{6d2})\",\"\u{686}\u{6cc}\u{646}\u{6cc} (\u{631}\u{648}\u{627}\u{6cc}\u{62a}\u{6cc}\u{60c}\u{6c1}\u{627}\u{646}\u{6af} \u{6a9}\u{627}\u{646}\u{6af})\",\"\u{633}\u{631}\u{628}\u{6cc}\u{646} (\u{633}\u{6cc}\u{631}\u{6cc}\u{644}\u{6a9}\u{60c}\u{645}\u{648}\u{646}\u{679}\u{6d2} \u{646}\u{6cc}\u{6af}\u{631}\u{648})\",\"xyz (\u{644}\u{627}\u{637}\u{6cc}\u{646}\u{6cc}\u{60c}\u{627}\u{645}\u{631}\u{6cc}\u{6a9}\u{627})\",\"\u{633}\u{648}\u{626}\u{633} \u{6c1}\u{627}\u{626}\u{6cc} \u{62c}\u{631}\u{645}\u{646} (1901)\",\"\u{627}\u{646}\u{6af}\u{631}\u{6cc}\u{632}\u{6cc} (\u{6cc}\u{648} \u{6a9}\u{6d2}) (OXENDICT)\",\"\u{644}\u{627}\u{637}\u{6cc}\u{646}\u{6cc} \u{627}\u{645}\u{631}\u{6cc}\u{6a9}\u{6cc} \u{6c1}\u{633}\u{67e}\u{627}\u{646}\u{648}\u{6cc}\",\"\u{628}\u{631}\u{627}\u{632}\u{6cc}\u{644}\u{6cc} \u{67e}\u{631}\u{62a}\u{6af}\u{627}\u{644}\u{6cc}\",\"root (\u{627}\u{645}\u{631}\u{6cc}\u{6a9}\u{627})\",\"\u{645}\u{627}\u{688}\u{631}\u{646} \u{627}\u{633}\u{679}\u{6cc}\u{646}\u{688}\u{631}\u{688} \u{639}\u{631}\u{628}\u{6cc}\",\"\u{641}\u{644}\u{6cc}\u{645}\u{650}\u{634}\",\"\u{6a9}\u{6cc}\u{646}\u{6cc}\u{688}\u{6cc}\u{646} \u{641}\u{631}\u{627}\u{646}\u{633}\u{6cc}\u{633}\u{6cc}\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('uz', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"en (Latn, UK)\",\"zh (Hant, Hong Kong)\",\"sr (Cyrl, Montenegro)\",\"xyz (Latn, US)\",\"de (Switzerland, 1901)\",\"en (UK, OXENDICT)\",\"es (Latin America)\",\"pt (Brazil)\",\"root (US)\",\"ar (001)\",\"nl (Belgium)\",\"fr (Canada)\"]",
        ),
        (
            "JSON.stringify(['US', 'DD', 'us', '419', '001', 'XK', 'EU'].map(c => new Intl.DisplayNames('vi', {type: 'region'}).of(c)))",
            "[\"Hoa K\u{1ef3}\",\"\u{110}\u{1ee9}c\",\"us\",\"Ch\u{e2}u M\u{1ef9} La-tinh\",\"001\",\"Kosovo\",\"Li\u{ea}n Minh Ch\u{e2}u \u{c2}u\"]",
        ),
        (
            "JSON.stringify(['gregory', 'japanese', 'islamic-civil', 'Gregory', 'abc'].map(c => new Intl.DisplayNames('zh', {type: 'calendar', style: 'short'}).of(c)))",
            "[\"\u{516c}\u{5386}\",\"\u{548c}\u{5386}\",\"\u{8868}\u{683c}\u{5f0f}\u{4f0a}\u{65af}\u{5170}\u{5386}\u{ff08}\u{6c11}\u{7528}\u{7eaa}\u{5143}\u{ff09}\",\"Gregory\",\"abc\"]",
        ),
        (
            "JSON.stringify(['era', 'weekOfYear', 'dayPeriod', 'timeZoneName'].map(c => new Intl.DisplayNames('de-AT', {type: 'dateTimeField', style: 'narrow'}).of(c)))",
            "[\"E\",\"W\",\"Tagesh.\",\"Zeitz.\"]",
        ),
        (
            "JSON.stringify(['usd', 'EUR', 'xyz', 'JPY', 'xxx'].map(c => new Intl.DisplayNames('en-GB', {type: 'currency', style: 'narrow'}).of(c)))",
            "[\"US Dollar\",\"Euro\",\"xyz\",\"Japanese Yen\",\"xxx\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('fr-CA', {type: 'language', style: 'long', languageDisplay: 'dialect'}).of(c)))",
            "[\"anglais (latin, Royaume-Uni)\",\"chinois (id\u{e9}ogrammes han traditionnels, Hong Kong)\",\"serbe (cyrillique, Mont\u{e9}n\u{e9}gro)\",\"xyz (latin, \u{c9}tats-Unis)\",\"allemand suisse (1901)\",\"anglais britannique (OXENDICT)\",\"espagnol d\u{2019}Am\u{e9}rique latine\",\"portugais br\u{e9}silien\",\"root (\u{c9}tats-Unis)\",\"arabe standard moderne\",\"flamand\",\"fran\u{e7}ais canadien\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('pt-PT', {type: 'language', style: 'long', languageDisplay: 'standard'}).of(c)))",
            "[\"ingl\u{ea}s (latim, Reino Unido)\",\"chin\u{ea}s (tradicional, Hong Kong)\",\"s\u{e9}rvio (cir\u{ed}lico, Montenegro)\",\"xyz (latim, Estados Unidos)\",\"alem\u{e3}o (Su\u{ed}\u{e7}a, 1901)\",\"ingl\u{ea}s (Reino Unido, OXENDICT)\",\"espanhol (Am\u{e9}rica Latina)\",\"portugu\u{ea}s (Brasil)\",\"root (Estados Unidos)\",\"\u{e1}rabe (001)\",\"neerland\u{ea}s (B\u{e9}lgica)\",\"franc\u{ea}s (Canad\u{e1})\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('zh-Hant', {type: 'language', style: 'short', languageDisplay: 'dialect'}).of(c)))",
            "[\"\u{82f1}\u{6587}\u{ff08}\u{62c9}\u{4e01}\u{5b57}\u{6bcd}\u{ff0c}\u{82f1}\u{570b}\u{ff09}\",\"\u{4e2d}\u{6587}\u{ff08}\u{7e41}\u{9ad4}\u{ff0c}\u{9999}\u{6e2f}\u{ff09}\",\"\u{585e}\u{723e}\u{7dad}\u{4e9e}\u{6587}\u{ff08}\u{897f}\u{91cc}\u{723e}\u{6587}\u{5b57}\u{ff0c}\u{8499}\u{7279}\u{5167}\u{54e5}\u{7f85}\u{ff09}\",\"xyz\u{ff08}\u{62c9}\u{4e01}\u{5b57}\u{6bcd}\u{ff0c}\u{7f8e}\u{570b}\u{ff09}\",\"\u{9ad8}\u{5730}\u{5fb7}\u{6587}\u{ff08}\u{745e}\u{58eb}\u{ff09}\u{ff08}1901\u{ff09}\",\"\u{82f1}\u{6587}\u{ff08}\u{82f1}\u{570b}\u{ff0c}\u{725b}\u{6d25}\u{82f1}\u{6587}\u{5b57}\u{5178}\u{62fc}\u{97f3}\u{ff09}\",\"\u{897f}\u{73ed}\u{7259}\u{6587}\u{ff08}\u{62c9}\u{4e01}\u{7f8e}\u{6d32}\u{ff09}\",\"\u{8461}\u{8404}\u{7259}\u{6587}\u{ff08}\u{5df4}\u{897f}\u{ff09}\",\"root\u{ff08}\u{7f8e}\u{570b}\u{ff09}\",\"\u{73fe}\u{4ee3}\u{6a19}\u{6e96}\u{963f}\u{62c9}\u{4f2f}\u{6587}\",\"\u{6cd5}\u{862d}\u{5fb7}\u{65af}\u{6587}\",\"\u{6cd5}\u{6587}\u{ff08}\u{52a0}\u{62ff}\u{5927}\u{ff09}\"]",
        ),
        (
            "JSON.stringify(['en-Latn-GB','zh-Hant-HK','sr-Cyrl-ME','xyz-Latn-US','de-CH-1901','en-GB-oxendict','es-419','pt-BR','und-US','ar-001','nl-BE','fr-CA'].map(c => new Intl.DisplayNames('sr-Latn', {type: 'language', style: 'short', languageDisplay: 'standard'}).of(c)))",
            "[\"engleski (latinica, UK)\",\"kineski (tradicionalno kinesko pismo, Hongkong)\",\"srpski (\u{107}irilica, Crna Gora)\",\"xyz (latinica, SAD)\",\"nema\u{10d}ki (\u{160}vajcarska, 1901)\",\"engleski (UK, OXENDICT)\",\"\u{161}panski (Latinska Amerika)\",\"portugalski (Brazil)\",\"root (SAD)\",\"arapski (001)\",\"holandski (Belgija)\",\"francuski (Kanada)\"]",
        ),
    ]);
}

#[test]
fn zones_before_1970_name_as_the_reference() {
    // Specific and generic names before ICU's metazones start, in 1970 (Chrome 156).
    check(&[
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'Europe/London', timeZoneName: 'long'})))",
            "[\"7/1/1950, 1:00:00 AM British Summer Time\",\"1/1/1950, 12:00:00 AM GMT+00:00\",\"7/1/1969, 1:00:00 AM GMT+01:00\",\"7/1/1972, 1:00:00 AM British Summer Time\",\"1/1/1880, 12:00:00 AM GMT+00:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'Europe/Dublin', timeZoneName: 'long'})))",
            "[\"7/1/1950, 1:00:00 AM Irish Standard Time\",\"1/1/1950, 12:00:00 AM GMT+00:00\",\"7/1/1969, 1:00:00 AM GMT+01:00\",\"7/1/1972, 1:00:00 AM Irish Standard Time\",\"12/31/1879, 11:34:39 PM GMT-00:25:21\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'Pacific/Honolulu', timeZoneName: 'long'})))",
            "[\"6/30/1950, 2:00:00 PM GMT-10:00\",\"12/31/1949, 2:00:00 PM GMT-10:00\",\"6/30/1969, 2:00:00 PM GMT-10:00\",\"6/30/1972, 2:00:00 PM Hawaii-Aleutian Standard Time\",\"12/31/1879, 1:28:34 PM GMT-10:31:26\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'America/New_York', timeZoneName: 'longGeneric'})))",
            "[\"6/30/1950, 8:00:00 PM New York Time\",\"12/31/1949, 7:00:00 PM New York Time\",\"6/30/1969, 8:00:00 PM New York Time\",\"6/30/1972, 8:00:00 PM Eastern Time\",\"12/31/1879, 7:03:58 PM New York Time\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'America/New_York', timeZoneName: 'shortGeneric'})))",
            "[\"6/30/1950, 8:00:00 PM New York Time\",\"12/31/1949, 7:00:00 PM New York Time\",\"6/30/1969, 8:00:00 PM New York Time\",\"6/30/1972, 8:00:00 PM ET\",\"12/31/1879, 7:03:58 PM New York Time\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('de', {timeZone: 'Europe/Berlin', timeZoneName: 'longGeneric'})))",
            "[\"1.7.1950, 01:00:00 Deutschland (Ortszeit)\",\"1.1.1950, 01:00:00 Deutschland (Ortszeit)\",\"1.7.1969, 01:00:00 Deutschland (Ortszeit)\",\"1.7.1972, 01:00:00 Mitteleurop\u{e4}ische Normalzeit\",\"1.1.1880, 00:53:28 Deutschland (Ortszeit)\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'Asia/Kolkata', timeZoneName: 'long'})))",
            "[\"7/1/1950, 5:30:00 AM GMT+05:30\",\"1/1/1950, 5:30:00 AM GMT+05:30\",\"7/1/1969, 5:30:00 AM GMT+05:30\",\"7/1/1972, 5:30:00 AM India Standard Time\",\"1/1/1880, 5:21:10 AM GMT+05:21:10\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'UTC', timeZoneName: 'long'})))",
            "[\"7/1/1950, 12:00:00 AM Coordinated Universal Time\",\"1/1/1950, 12:00:00 AM Coordinated Universal Time\",\"7/1/1969, 12:00:00 AM Coordinated Universal Time\",\"7/1/1972, 12:00:00 AM Coordinated Universal Time\",\"1/1/1880, 12:00:00 AM Coordinated Universal Time\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('ja', {timeZone: 'Asia/Tokyo', timeZoneName: 'short'})))",
            "[\"1950/7/1 10:00:00 GMT+10\",\"1950/1/1 9:00:00 GMT+9\",\"1969/7/1 9:00:00 GMT+9\",\"1972/7/1 9:00:00 JST\",\"1880/1/1 9:18:59 GMT+9:18:59\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'Australia/Sydney', timeZoneName: 'short'})))",
            "[\"7/1/1950, 10:00:00 AM GMT+10\",\"1/1/1950, 10:00:00 AM GMT+10\",\"7/1/1969, 10:00:00 AM GMT+10\",\"7/1/1972, 10:00:00 AM GMT+10\",\"1/1/1880, 10:04:52 AM GMT+10:04:52\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en-GB', {timeZone: 'Europe/London', timeZoneName: 'short'})))",
            "[\"01/07/1950, 1:00:00 BST\",\"01/01/1950, 0:00:00 GMT+0\",\"01/07/1969, 1:00:00 GMT+1\",\"01/07/1972, 1:00:00 BST\",\"01/01/1880, 0:00:00 GMT+0\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1950, 6, 1), Date.UTC(1950, 0, 1), Date.UTC(1969, 6, 1), Date.UTC(1972, 6, 1), Date.UTC(1880, 0, 1)].map(t => new Date(t).toLocaleString('en', {timeZone: 'America/Los_Angeles'})))",
            "[\"6/30/1950, 5:00:00 PM\",\"12/31/1949, 4:00:00 PM\",\"6/30/1969, 5:00:00 PM\",\"6/30/1972, 5:00:00 PM\",\"12/31/1879, 4:07:02 PM\"]",
        ),
    ]);
}

#[test]
fn zones_before_1970_keep_their_history() {
    // Each zone's offsets through its history in zoneinfo64, local mean time and seconds included (Chrome 156), a sample of 457 zones.
    check(&[
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Africa/Abidjan', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 11:43:52 PM GMT-00:16:08\",\"6/1/1890, 11:43:52 AM GMT-00:16:08\",\"7/15/1910, 2:43:52 AM GMT-00:16:08\",\"1/10/1925, 12:00:00 AM GMT+00:00\",\"8/1/1942, 6:00:00 PM GMT+00:00\",\"4/20/1955, 6:00:00 AM GMT+00:00\",\"10/31/1965, 11:00:00 PM GMT+00:00\",\"12/31/1969, 11:30:00 PM GMT+00:00\",\"6/1/1975, 12:00:00 AM GMT+00:00\",\"7/1/2024, 12:00:00 AM GMT+00:00\",\"6/30/1800, 11:43:52 PM GMT-00:16:08\",\"2/10/1653, 5:57:12 AM GMT-00:16:08\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Africa/Harare', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 2:10:18 AM GMT+02:10:18\",\"6/1/1890, 2:10:18 PM GMT+02:10:18\",\"7/15/1910, 5:00:00 AM GMT+02:00\",\"1/10/1925, 2:00:00 AM GMT+02:00\",\"8/1/1942, 8:00:00 PM GMT+02:00\",\"4/20/1955, 8:00:00 AM GMT+02:00\",\"11/1/1965, 1:00:00 AM GMT+02:00\",\"1/1/1970, 1:30:00 AM GMT+02:00\",\"6/1/1975, 2:00:00 AM GMT+02:00\",\"7/1/2024, 2:00:00 AM GMT+02:00\",\"7/1/1800, 2:10:18 AM GMT+02:10:18\",\"2/10/1653, 8:23:38 AM GMT+02:10:18\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Africa/Ouagadougou', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 11:43:52 PM GMT-00:16:08\",\"6/1/1890, 11:43:52 AM GMT-00:16:08\",\"7/15/1910, 2:43:52 AM GMT-00:16:08\",\"1/10/1925, 12:00:00 AM GMT+00:00\",\"8/1/1942, 6:00:00 PM GMT+00:00\",\"4/20/1955, 6:00:00 AM GMT+00:00\",\"10/31/1965, 11:00:00 PM GMT+00:00\",\"12/31/1969, 11:30:00 PM GMT+00:00\",\"6/1/1975, 12:00:00 AM GMT+00:00\",\"7/1/2024, 12:00:00 AM GMT+00:00\",\"6/30/1800, 11:43:52 PM GMT-00:16:08\",\"2/10/1653, 5:57:12 AM GMT-00:16:08\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/Belem', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 8:46:04 PM GMT-03:13:56\",\"6/1/1890, 8:46:04 AM GMT-03:13:56\",\"7/14/1910, 11:46:04 PM GMT-03:13:56\",\"1/9/1925, 9:00:00 PM GMT-03:00\",\"8/1/1942, 3:00:00 PM GMT-03:00\",\"4/20/1955, 3:00:00 AM GMT-03:00\",\"10/31/1965, 8:00:00 PM GMT-03:00\",\"12/31/1969, 8:30:00 PM GMT-03:00\",\"5/31/1975, 9:00:00 PM GMT-03:00\",\"6/30/2024, 9:00:00 PM GMT-03:00\",\"6/30/1800, 8:46:04 PM GMT-03:13:56\",\"2/10/1653, 2:59:24 AM GMT-03:13:56\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/Curacao', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 7:35:35 PM GMT-04:24:25\",\"6/1/1890, 7:35:35 AM GMT-04:24:25\",\"7/14/1910, 11:00:00 PM GMT-04:00\",\"1/9/1925, 8:00:00 PM GMT-04:00\",\"8/1/1942, 3:00:00 PM GMT-03:00\",\"4/20/1955, 2:00:00 AM GMT-04:00\",\"10/31/1965, 7:00:00 PM GMT-04:00\",\"12/31/1969, 7:30:00 PM GMT-04:00\",\"5/31/1975, 8:00:00 PM GMT-04:00\",\"6/30/2024, 8:00:00 PM GMT-04:00\",\"6/30/1800, 7:35:35 PM GMT-04:24:25\",\"2/10/1653, 1:48:55 AM GMT-04:24:25\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/Hermosillo', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 4:36:08 PM GMT-07:23:52\",\"6/1/1890, 4:36:08 AM GMT-07:23:52\",\"7/14/1910, 7:36:08 PM GMT-07:23:52\",\"1/9/1925, 5:00:00 PM GMT-07:00\",\"8/1/1942, 11:00:00 AM GMT-07:00\",\"4/19/1955, 11:00:00 PM GMT-07:00\",\"10/31/1965, 4:00:00 PM GMT-07:00\",\"12/31/1969, 4:30:00 PM GMT-07:00\",\"5/31/1975, 5:00:00 PM GMT-07:00\",\"6/30/2024, 5:00:00 PM GMT-07:00\",\"6/30/1800, 4:36:08 PM GMT-07:23:52\",\"2/9/1653, 10:49:28 PM GMT-07:23:52\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/Manaus', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 7:59:56 PM GMT-04:00:04\",\"6/1/1890, 7:59:56 AM GMT-04:00:04\",\"7/14/1910, 10:59:56 PM GMT-04:00:04\",\"1/9/1925, 8:00:00 PM GMT-04:00\",\"8/1/1942, 2:00:00 PM GMT-04:00\",\"4/20/1955, 2:00:00 AM GMT-04:00\",\"10/31/1965, 7:00:00 PM GMT-04:00\",\"12/31/1969, 7:30:00 PM GMT-04:00\",\"5/31/1975, 8:00:00 PM GMT-04:00\",\"6/30/2024, 8:00:00 PM GMT-04:00\",\"6/30/1800, 7:59:56 PM GMT-04:00:04\",\"2/10/1653, 2:13:16 AM GMT-04:00:04\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/Panama', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 6:41:52 PM GMT-05:18:08\",\"6/1/1890, 6:40:24 AM GMT-05:19:36\",\"7/14/1910, 10:00:00 PM GMT-05:00\",\"1/9/1925, 7:00:00 PM GMT-05:00\",\"8/1/1942, 1:00:00 PM GMT-05:00\",\"4/20/1955, 1:00:00 AM GMT-05:00\",\"10/31/1965, 6:00:00 PM GMT-05:00\",\"12/31/1969, 6:30:00 PM GMT-05:00\",\"5/31/1975, 7:00:00 PM GMT-05:00\",\"6/30/2024, 7:00:00 PM GMT-05:00\",\"6/30/1800, 6:41:52 PM GMT-05:18:08\",\"2/10/1653, 12:55:12 AM GMT-05:18:08\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'America/St_Thomas', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 7:35:35 PM GMT-04:24:25\",\"6/1/1890, 7:35:35 AM GMT-04:24:25\",\"7/14/1910, 11:00:00 PM GMT-04:00\",\"1/9/1925, 8:00:00 PM GMT-04:00\",\"8/1/1942, 3:00:00 PM GMT-03:00\",\"4/20/1955, 2:00:00 AM GMT-04:00\",\"10/31/1965, 7:00:00 PM GMT-04:00\",\"12/31/1969, 7:30:00 PM GMT-04:00\",\"5/31/1975, 8:00:00 PM GMT-04:00\",\"6/30/2024, 8:00:00 PM GMT-04:00\",\"6/30/1800, 7:35:35 PM GMT-04:24:25\",\"2/10/1653, 1:48:55 AM GMT-04:24:25\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Arctic/Longyearbyen', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 12:53:28 AM GMT+00:53:28\",\"6/1/1890, 12:53:28 PM GMT+00:53:28\",\"7/15/1910, 4:00:00 AM GMT+01:00\",\"1/10/1925, 1:00:00 AM GMT+01:00\",\"8/1/1942, 8:00:00 PM GMT+02:00\",\"4/20/1955, 7:00:00 AM GMT+01:00\",\"11/1/1965, 12:00:00 AM GMT+01:00\",\"1/1/1970, 12:30:00 AM GMT+01:00\",\"6/1/1975, 1:00:00 AM GMT+01:00\",\"7/1/2024, 2:00:00 AM GMT+02:00\",\"7/1/1800, 12:53:28 AM GMT+00:53:28\",\"2/10/1653, 7:06:48 AM GMT+00:53:28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Asia/Dubai', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 3:41:12 AM GMT+03:41:12\",\"6/1/1890, 3:41:12 PM GMT+03:41:12\",\"7/15/1910, 6:41:12 AM GMT+03:41:12\",\"1/10/1925, 4:00:00 AM GMT+04:00\",\"8/1/1942, 10:00:00 PM GMT+04:00\",\"4/20/1955, 10:00:00 AM GMT+04:00\",\"11/1/1965, 3:00:00 AM GMT+04:00\",\"1/1/1970, 3:30:00 AM GMT+04:00\",\"6/1/1975, 4:00:00 AM GMT+04:00\",\"7/1/2024, 4:00:00 AM GMT+04:00\",\"7/1/1800, 3:41:12 AM GMT+03:41:12\",\"2/10/1653, 9:54:32 AM GMT+03:41:12\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Asia/Manila', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 8:03:52 AM GMT+08:03:52\",\"6/1/1890, 8:03:52 PM GMT+08:03:52\",\"7/15/1910, 11:00:00 AM GMT+08:00\",\"1/10/1925, 8:00:00 AM GMT+08:00\",\"8/2/1942, 3:00:00 AM GMT+09:00\",\"4/20/1955, 2:00:00 PM GMT+08:00\",\"11/1/1965, 7:00:00 AM GMT+08:00\",\"1/1/1970, 7:30:00 AM GMT+08:00\",\"6/1/1975, 8:00:00 AM GMT+08:00\",\"7/1/2024, 8:00:00 AM GMT+08:00\",\"6/30/1800, 8:03:52 AM GMT-15:56:08\",\"2/9/1653, 2:17:12 PM GMT-15:56:08\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Asia/Tashkent', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 4:37:11 AM GMT+04:37:11\",\"6/1/1890, 4:37:11 PM GMT+04:37:11\",\"7/15/1910, 7:37:11 AM GMT+04:37:11\",\"1/10/1925, 5:00:00 AM GMT+05:00\",\"8/2/1942, 12:00:00 AM GMT+06:00\",\"4/20/1955, 12:00:00 PM GMT+06:00\",\"11/1/1965, 5:00:00 AM GMT+06:00\",\"1/1/1970, 5:30:00 AM GMT+06:00\",\"6/1/1975, 6:00:00 AM GMT+06:00\",\"7/1/2024, 5:00:00 AM GMT+05:00\",\"7/1/1800, 4:37:11 AM GMT+04:37:11\",\"2/10/1653, 10:50:31 AM GMT+04:37:11\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Atlantic/Stanley', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 8:08:36 PM GMT-03:51:24\",\"6/1/1890, 8:08:36 AM GMT-03:51:24\",\"7/14/1910, 11:08:36 PM GMT-03:51:24\",\"1/9/1925, 8:00:00 PM GMT-04:00\",\"8/1/1942, 2:00:00 PM GMT-04:00\",\"4/20/1955, 2:00:00 AM GMT-04:00\",\"10/31/1965, 7:00:00 PM GMT-04:00\",\"12/31/1969, 7:30:00 PM GMT-04:00\",\"5/31/1975, 8:00:00 PM GMT-04:00\",\"6/30/2024, 9:00:00 PM GMT-03:00\",\"6/30/1800, 8:08:36 PM GMT-03:51:24\",\"2/10/1653, 2:21:56 AM GMT-03:51:24\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Etc/GMT+6', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"12/31/1849, 6:00:00 PM GMT-06:00\",\"6/1/1890, 6:00:00 AM GMT-06:00\",\"7/14/1910, 9:00:00 PM GMT-06:00\",\"1/9/1925, 6:00:00 PM GMT-06:00\",\"8/1/1942, 12:00:00 PM GMT-06:00\",\"4/20/1955, 12:00:00 AM GMT-06:00\",\"10/31/1965, 5:00:00 PM GMT-06:00\",\"12/31/1969, 5:30:00 PM GMT-06:00\",\"5/31/1975, 6:00:00 PM GMT-06:00\",\"6/30/2024, 6:00:00 PM GMT-06:00\",\"6/30/1800, 6:00:00 PM GMT-06:00\",\"2/10/1653, 12:13:20 AM GMT-06:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Europe/Astrakhan', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 3:12:12 AM GMT+03:12:12\",\"6/1/1890, 3:12:12 PM GMT+03:12:12\",\"7/15/1910, 6:12:12 AM GMT+03:12:12\",\"1/10/1925, 3:00:00 AM GMT+03:00\",\"8/1/1942, 10:00:00 PM GMT+04:00\",\"4/20/1955, 10:00:00 AM GMT+04:00\",\"11/1/1965, 3:00:00 AM GMT+04:00\",\"1/1/1970, 3:30:00 AM GMT+04:00\",\"6/1/1975, 4:00:00 AM GMT+04:00\",\"7/1/2024, 4:00:00 AM GMT+04:00\",\"7/1/1800, 3:12:12 AM GMT+03:12:12\",\"2/10/1653, 9:25:32 AM GMT+03:12:12\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Europe/London', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 12:00:00 AM GMT+00:00\",\"6/1/1890, 12:00:00 PM GMT+00:00\",\"7/15/1910, 3:00:00 AM GMT+00:00\",\"1/10/1925, 12:00:00 AM GMT+00:00\",\"8/1/1942, 8:00:00 PM GMT+02:00\",\"4/20/1955, 7:00:00 AM GMT+01:00\",\"10/31/1965, 11:00:00 PM GMT+00:00\",\"1/1/1970, 12:30:00 AM GMT+01:00\",\"6/1/1975, 1:00:00 AM GMT+01:00\",\"7/1/2024, 1:00:00 AM GMT+01:00\",\"6/30/1800, 11:58:45 PM GMT-00:01:15\",\"2/10/1653, 6:12:05 AM GMT-00:01:15\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Europe/Tirane', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 1:19:20 AM GMT+01:19:20\",\"6/1/1890, 1:19:20 PM GMT+01:19:20\",\"7/15/1910, 4:19:20 AM GMT+01:19:20\",\"1/10/1925, 1:00:00 AM GMT+01:00\",\"8/1/1942, 8:00:00 PM GMT+02:00\",\"4/20/1955, 7:00:00 AM GMT+01:00\",\"11/1/1965, 12:00:00 AM GMT+01:00\",\"1/1/1970, 12:30:00 AM GMT+01:00\",\"6/1/1975, 2:00:00 AM GMT+02:00\",\"7/1/2024, 2:00:00 AM GMT+02:00\",\"7/1/1800, 1:19:20 AM GMT+01:19:20\",\"2/10/1653, 7:32:40 AM GMT+01:19:20\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Pacific/Apia', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 12:33:04 PM GMT+12:33:04\",\"6/2/1890, 12:33:04 AM GMT+12:33:04\",\"7/14/1910, 3:33:04 PM GMT-11:26:56\",\"1/9/1925, 12:30:00 PM GMT-11:30\",\"8/1/1942, 6:30:00 AM GMT-11:30\",\"4/19/1955, 7:00:00 PM GMT-11:00\",\"10/31/1965, 12:00:00 PM GMT-11:00\",\"12/31/1969, 12:30:00 PM GMT-11:00\",\"5/31/1975, 1:00:00 PM GMT-11:00\",\"7/1/2024, 1:00:00 PM GMT+13:00\",\"7/1/1800, 12:33:04 PM GMT+12:33:04\",\"2/10/1653, 6:46:24 PM GMT+12:33:04\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => { try { return new Date(t).toLocaleString('en-US', {timeZone: 'Pacific/Norfolk', timeZoneName: 'longOffset'}); } catch (e) { return e.name; } }))",
            "[\"1/1/1850, 11:11:52 AM GMT+11:11:52\",\"6/1/1890, 11:11:52 PM GMT+11:11:52\",\"7/15/1910, 2:12:00 PM GMT+11:12\",\"1/10/1925, 11:12:00 AM GMT+11:12\",\"8/2/1942, 5:12:00 AM GMT+11:12\",\"4/20/1955, 5:30:00 PM GMT+11:30\",\"11/1/1965, 10:30:00 AM GMT+11:30\",\"1/1/1970, 11:00:00 AM GMT+11:30\",\"6/1/1975, 11:30:00 AM GMT+11:30\",\"7/1/2024, 11:00:00 AM GMT+11:00\",\"7/1/1800, 11:11:52 AM GMT+11:11:52\",\"2/10/1653, 5:25:12 PM GMT+11:11:52\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => new Date(t).toLocaleString('de', {timeZone: 'Europe/Berlin', timeZoneName: 'short'})))",
            "[\"1.1.1850, 0:53:28 GMT+0:53:28\",\"1.6.1890, 12:53:28 GMT+0:53:28\",\"15.7.1910, 4:00:00 GMT+1\",\"10.1.1925, 1:00:00 GMT+1\",\"1.8.1942, 20:00:00 GMT+2\",\"20.4.1955, 7:00:00 GMT+1\",\"1.11.1965, 0:00:00 GMT+1\",\"1.1.1970, 0:30:00 GMT+1\",\"1.6.1975, 1:00:00 MEZ\",\"1.7.2024, 2:00:00 MESZ\",\"1.7.1800, 0:53:28 GMT+0:53:28\",\"10.2.1653, 7:06:48 GMT+0:53:28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => new Date(t).toLocaleString('en', {timeZone: 'America/New_York', timeZoneName: 'long'})))",
            "[\"12/31/1849, 7:03:58 PM GMT-04:56:02\",\"6/1/1890, 7:00:00 AM GMT-05:00\",\"7/14/1910, 10:00:00 PM GMT-05:00\",\"1/9/1925, 7:00:00 PM GMT-05:00\",\"8/1/1942, 2:00:00 PM GMT-04:00\",\"4/20/1955, 1:00:00 AM GMT-05:00\",\"10/31/1965, 6:00:00 PM GMT-05:00\",\"12/31/1969, 6:30:00 PM GMT-05:00\",\"5/31/1975, 8:00:00 PM Eastern Daylight Time\",\"6/30/2024, 8:00:00 PM Eastern Daylight Time\",\"6/30/1800, 7:03:58 PM GMT-04:56:02\",\"2/10/1653, 1:17:18 AM GMT-04:56:02\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => new Date(t).toLocaleString('ja', {timeZone: 'Asia/Tokyo', timeZoneName: 'shortOffset'})))",
            "[\"1850/1/1 9:18:59 GMT+9:18:59\",\"1890/6/1 21:00:00 GMT+9\",\"1910/7/15 12:00:00 GMT+9\",\"1925/1/10 9:00:00 GMT+9\",\"1942/8/2 3:00:00 GMT+9\",\"1955/4/20 15:00:00 GMT+9\",\"1965/11/1 8:00:00 GMT+9\",\"1970/1/1 8:30:00 GMT+9\",\"1975/6/1 9:00:00 GMT+9\",\"2024/7/1 9:00:00 GMT+9\",\"1800/7/1 9:18:59 GMT+9:18:59\",\"1653/2/10 15:32:19 GMT+9:18:59\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => new Date(t).toLocaleString('en', {timeZone: 'Europe/Amsterdam', timeZoneName: 'longOffset'})))",
            "[\"1/1/1850, 12:17:30 AM GMT+00:17:30\",\"6/1/1890, 12:17:30 PM GMT+00:17:30\",\"7/15/1910, 3:00:00 AM GMT+00:00\",\"1/10/1925, 12:00:00 AM GMT+00:00\",\"8/1/1942, 8:00:00 PM GMT+02:00\",\"4/20/1955, 7:00:00 AM GMT+01:00\",\"11/1/1965, 12:00:00 AM GMT+01:00\",\"1/1/1970, 12:30:00 AM GMT+01:00\",\"6/1/1975, 1:00:00 AM GMT+01:00\",\"7/1/2024, 2:00:00 AM GMT+02:00\",\"7/1/1800, 12:17:30 AM GMT+00:17:30\",\"2/10/1653, 6:30:50 AM GMT+00:17:30\"]",
        ),
        (
            "JSON.stringify([Date.UTC(1850, 0, 1), Date.UTC(1890, 5, 1, 12), Date.UTC(1910, 6, 15, 3), Date.UTC(1925, 0, 10), Date.UTC(1942, 7, 1, 18), Date.UTC(1955, 3, 20, 6), Date.UTC(1965, 9, 31, 23), Date.UTC(1969, 11, 31, 23, 30), Date.UTC(1975, 5, 1), Date.UTC(2024, 6, 1), Date.UTC(1800, 6, 1), -1e13].map(t => new Date(t).toLocaleString('fr', {timeZone: 'Europe/Paris', timeZoneName: 'shortOffset'})))",
            "[\"01/01/1850 00:09:21 UTC+0:09:21\",\"01/06/1890 12:09:21 UTC+0:09:21\",\"15/07/1910 03:09:21 UTC+0:09:21\",\"10/01/1925 00:00:00 UTC+0\",\"01/08/1942 20:00:00 UTC+2\",\"20/04/1955 07:00:00 UTC+1\",\"01/11/1965 00:00:00 UTC+1\",\"01/01/1970 00:30:00 UTC+1\",\"01/06/1975 01:00:00 UTC+1\",\"01/07/2024 02:00:00 UTC+2\",\"01/07/1800 00:09:21 UTC+0:09:21\",\"10/02/1653 06:22:41 UTC+0:09:21\"]",
        ),
    ]);
}

#[test]
fn calendars_write_as_the_reference() {
    // Each calendar a -u-ca- asks for, by the reference browser's patterns of it (Chrome 156): a sample of the 10,200 lists compared.
    check(&[
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('af-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"15/1/2567 BE\",\"1/6/2566 BE\",\"31/12/2533 BE\",\"23/10/2411 BE\",\"20/3/2574 BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('af-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"7/1445 AH\",\"11/1444 AH\",\"6/1411 AH\",\"7/1285 AH\",\"11/1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ak-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"2023 M12 5\",\"2023 M04 13\",\"1990 M11 15\",\"1868 M09 8\",\"2031 M02 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ak-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"1445/7/3\",\"1444/11/12\",\"1411/6/13\",\"1285/7/6\",\"1452/11/26\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('am-u-ca-gregory', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"\u{1303}\u{1295} 15 2024 \u{12d3}/\u{121d}\",\"\u{1301}\u{1295} 1 2023 \u{12d3}/\u{121d}\",\"\u{12f2}\u{1234}\u{121d} 31 1990 \u{12d3}/\u{121d}\",\"\u{12a6}\u{12ad}\u{1276} 23 1868 \u{12d3}/\u{121d}\",\"\u{121b}\u{122d}\u{127d} 20 2031 \u{12d3}/\u{121d}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ar-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"15 \u{64a}\u{646}\u{627}\u{64a}\u{631} 2567 BE\",\"1 \u{64a}\u{648}\u{646}\u{64a}\u{648} 2566 BE\",\"31 \u{62f}\u{64a}\u{633}\u{645}\u{628}\u{631} 2533 BE\",\"23 \u{623}\u{643}\u{62a}\u{648}\u{628}\u{631} 2411 BE\",\"20 \u{645}\u{627}\u{631}\u{633} 2574 BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ar-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"4 \u{631}\u{62c}\u{628} 1445 \u{647}\u{640}\u{60c} 12:00 \u{635}\",\"13 \u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}\u{629} 1444 \u{647}\u{640}\u{60c} 12:00 \u{635}\",\"14 \u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{622}\u{62e}\u{631}\u{629} 1411 \u{647}\u{640}\u{60c} 12:00 \u{635}\",\"8 \u{631}\u{62c}\u{628} 1285 \u{647}\u{640}\u{60c} 12:00 \u{635}\",\"27 \u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}\u{629} 1452 \u{647}\u{640}\u{60c} 12:00 \u{635}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('az-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"2023(gui-mao) M12 5\",\"2023(gui-mao) M04 13\",\"1990(geng-wu) M11 15\",\"1868(wu-chen) M09 8\",\"2031(xin-hai) M02 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('az-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"AH 1445 Rajab 3\",\"AH 1444 Dhu\u{2bb}l-Qi\u{2bb}dah 12\",\"AH 1411 Jumada II 13\",\"AH 1285 Rajab 6\",\"AH 1452 Dhu\u{2bb}l-Qi\u{2bb}dah 26\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bg-u-ca-gregory', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"15 \u{44f}\u{43d}\u{443}\u{430}\u{440}\u{438}\",\"1 \u{44e}\u{43d}\u{438}\",\"31 \u{434}\u{435}\u{43a}\u{435}\u{43c}\u{432}\u{440}\u{438}\",\"23 \u{43e}\u{43a}\u{442}\u{43e}\u{43c}\u{432}\u{440}\u{438}\",\"20 \u{43c}\u{430}\u{440}\u{442}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bn-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"\u{9e7}\u{9eb}/\u{9e7}/\u{9e8}\u{9eb}\u{9ec}\u{9ed} BE\",\"\u{9e7}/\u{9ec}/\u{9e8}\u{9eb}\u{9ec}\u{9ec} BE\",\"\u{9e9}\u{9e7}/\u{9e7}\u{9e8}/\u{9e8}\u{9eb}\u{9e9}\u{9e9} BE\",\"\u{9e8}\u{9e9}/\u{9e7}\u{9e6}/\u{9e8}\u{9ea}\u{9e7}\u{9e7} BE\",\"\u{9e8}\u{9e6}/\u{9e9}/\u{9e8}\u{9eb}\u{9ed}\u{9ea} BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bn-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"\u{9b8}\u{9cb}\u{9ae}\u{9ac}\u{9be}\u{9b0}, \u{9ea} \u{9b0}\u{99c}\u{9ac}, \u{9e7}\u{9ea}\u{9ea}\u{9eb} \u{9af}\u{9c1}\u{997}\",\"\u{9ac}\u{9c3}\u{9b9}\u{9b8}\u{9cd}\u{9aa}\u{9a4}\u{9bf}\u{9ac}\u{9be}\u{9b0}, \u{9e7}\u{9e8} \u{99c}\u{9cd}\u{9ac}\u{9bf}\u{9b2}\u{995}\u{9a6}, \u{9e7}\u{9ea}\u{9ea}\u{9ea} \u{9af}\u{9c1}\u{997}\",\"\u{9b8}\u{9cb}\u{9ae}\u{9ac}\u{9be}\u{9b0}, \u{9e7}\u{9e9} \u{99c}\u{9ae}\u{9be}\u{9a6}\u{9bf}\u{989}\u{9b8} \u{9b8}\u{9be}\u{9a8}\u{9bf}, \u{9e7}\u{9ea}\u{9e7}\u{9e7} \u{9af}\u{9c1}\u{997}\",\"\u{9b6}\u{9c1}\u{995}\u{9cd}\u{9b0}\u{9ac}\u{9be}\u{9b0}, \u{9ec} \u{9b0}\u{99c}\u{9ac}, \u{9e7}\u{9e8}\u{9ee}\u{9eb} \u{9af}\u{9c1}\u{997}\",\"\u{9ac}\u{9c3}\u{9b9}\u{9b8}\u{9cd}\u{9aa}\u{9a4}\u{9bf}\u{9ac}\u{9be}\u{9b0}, \u{9e8}\u{9ec} \u{99c}\u{9cd}\u{9ac}\u{9bf}\u{9b2}\u{995}\u{9a6}, \u{9e7}\u{9ea}\u{9eb}\u{9e8} \u{9af}\u{9c1}\u{997}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bs-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"2023-12\",\"2023-04\",\"1990-11\",\"1868-09\",\"2031-02\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bs-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"AH 1445-07-03\",\"AH 1444-11-12\",\"AH 1411-06-13\",\"AH 1285-07-06\",\"AH 1452-11-26\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ca-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"5/5/5784 AM\",\"12/9/5783 AM\",\"14/4/5751 AM\",\"7/2/5629 AM\",\"25/6/5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('cs-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"15. 1. 2567 BE\",\"1. 6. 2566 BE\",\"31. 12. 2533 BE\",\"23. 10. 2411 BE\",\"20. 3. 2574 BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('cs-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"04.07.45 AH\",\"12.11.44 AH\",\"13.06.11 AH\",\"06.07.85 AH\",\"26.11.52 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('da-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"6.5.7516\",\"24.9.7515\",\"22.4.7483\",\"14.2.7361\",\"11.7.7523\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('da-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"7.1445 AH\",\"11.1444 AH\",\"6.1411 AH\",\"7.1285 AH\",\"11.1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('de-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"05.05.5784 AM\",\"12.09.5783 AM\",\"14.04.5751 AM\",\"07.02.5629 AM\",\"25.06.5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('el-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"15 \u{399}\u{3b1}\u{3bd} 2567 BE, 12:00 \u{3c0}.\u{3bc}.\",\"1 \u{399}\u{3bf}\u{3c5}\u{3bd} 2566 BE, 12:00 \u{3c0}.\u{3bc}.\",\"31 \u{394}\u{3b5}\u{3ba} 2533 BE, 12:00 \u{3c0}.\u{3bc}.\",\"23 \u{39f}\u{3ba}\u{3c4} 2411 BE, 12:00 \u{3c0}.\u{3bc}.\",\"20 \u{39c}\u{3b1}\u{3c1} 2574 BE, 12:00 \u{3c0}.\u{3bc}.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('el-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"4 Raj. 1445 \u{395}.\u{395}.\",\"12 Dhu\u{2bb}l-Q. 1444 \u{395}.\u{395}.\",\"13 Jum. II 1411 \u{395}.\u{395}.\",\"6 Raj. 1285 \u{395}.\u{395}.\",\"26 Dhu\u{2bb}l-Q. 1452 \u{395}.\u{395}.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('en-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"Ter 6, 7516 AA\",\"Genbot 24, 7515 AA\",\"Tahsas 22, 7483 AA\",\"Tekemt 14, 7361 AA\",\"Megabit 11, 7523 AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('en-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"Raj. 3, 1445 AH, 12:00 AM\",\"Dhu\u{2bb}l-Q. 12, 1444 AH, 12:00 AM\",\"Jum. II 13, 1411 AH, 12:00 AM\",\"Raj. 6, 1285 AH, 12:00 AM\",\"Dhu\u{2bb}l-Q. 26, 1452 AH, 12:00 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('es-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"5 de shevat de 5784 AM\",\"12 de sivan de 5783 AM\",\"14 de tevet de 5751 AM\",\"7 de heshvan de 5629 AM\",\"25 de adar de 5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('et-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"2023(gui-mao) kaheteistk\u{fc}mnes kuu 5, esmasp\u{e4}ev\",\"2023(gui-mao) neljas kuu 14, neljap\u{e4}ev\",\"1990(geng-wu) \u{fc}heteistk\u{fc}mnes kuu 15, esmasp\u{e4}ev\",\"1868(wu-chen) \u{fc}heksas kuu 8, reede\",\"2031(xin-hai) teine kuu 28, neljap\u{e4}ev\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('et-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"4. Rajab\",\"12. Dhu\u{2bb}l-Qi\u{2bb}dah\",\"13. Jumada II\",\"6. Rajab\",\"26. Dhu\u{2bb}l-Qi\u{2bb}dah\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fa-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"\u{6f7}\u{6f5}\u{6f1}\u{6f6}/\u{6f5}/\u{6f6} AA\",\"\u{6f7}\u{6f5}\u{6f1}\u{6f5}/\u{6f9}/\u{6f2}\u{6f4} AA\",\"\u{6f7}\u{6f4}\u{6f8}\u{6f3}/\u{6f4}/\u{6f2}\u{6f2} AA\",\"\u{6f7}\u{6f3}\u{6f6}\u{6f1}/\u{6f2}/\u{6f1}\u{6f4} AA\",\"\u{6f7}\u{6f5}\u{6f2}\u{6f3}/\u{6f7}/\u{6f1}\u{6f1} AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fa-u-ca-persian', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"\u{6f1}\u{6f4}\u{6f0}\u{6f2}/\u{6f1}\u{6f0}/\u{6f2}\u{6f5}\",\"\u{6f1}\u{6f4}\u{6f0}\u{6f2}/\u{6f3}/\u{6f1}\u{6f1}\",\"\u{6f1}\u{6f3}\u{6f6}\u{6f9}/\u{6f1}\u{6f0}/\u{6f1}\u{6f0}\",\"\u{6f1}\u{6f2}\u{6f4}\u{6f7}/\u{6f8}/\u{6f2}\",\"\u{6f1}\u{6f4}\u{6f0}\u{6f9}/\u{6f1}\u{6f2}/\u{6f2}\u{6f9}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fi-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"5.5784 AM\",\"9.5783 AM\",\"4.5751 AM\",\"2.5629 AM\",\"6.5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fil-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"2023 M12 5\",\"2023 M04 14\",\"1990 M11 15\",\"1868 M09 8\",\"2031 M02 28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fil-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"7/4/1445 AH\",\"11/13/1444 AH\",\"6/14/1411 AH\",\"7/8/1285 AH\",\"11/27/1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fr-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"6 ter 7516 AA\",\"24 gue. 7515 AA\",\"22 tah. 7483 AA\",\"14 teq. 7361 AA\",\"11 m\u{e4}g. 7523 AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('fr-u-ca-persian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"25 dey 1402 A. P.\",\"11 khord\u{e2}d 1402 A. P.\",\"10 dey 1369 A. P.\",\"2 \u{e2}b\u{e2}n 1247 A. P.\",\"29 esfand 1409 A. P.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('gu-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"5 \u{ab6}\u{ac7}\u{ab5}\u{abe}\u{aa4}, AM 5784 12:00 AM\",\"12 \u{ab8}\u{abf}\u{ab5}\u{abe}\u{aa8}, AM 5783 12:00 AM\",\"14 \u{aa4}\u{ac7}\u{ab5}\u{ac7}\u{a9f}, AM 5751 12:00 AM\",\"7 \u{ab9}\u{ac7}\u{ab6}\u{ab5}\u{abe}\u{aa8}, AM 5629 12:00 AM\",\"25 \u{a85}\u{aa6}\u{abe}\u{ab0}, AM 5791 12:00 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('he-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"2023(gui-mao) M12 5\",\"2023(gui-mao) M04 14\",\"1990(geng-wu) M11 15\",\"1868(wu-chen) M09 8\",\"2031(xin-hai) M02 28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('he-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"7/1445\",\"11/1444\",\"6/1411\",\"7/1285\",\"11/1452\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('hi-u-ca-ethiopic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"AM 6 \u{91f}\u{930} 2016\",\"AM 24 \u{917}\u{928}\u{92c}\u{94b}\u{91f} 2015\",\"AM 22 \u{924}\u{939}\u{938}\u{93e}\u{938} 1983\",\"AM 14 \u{91f}\u{947}\u{915}\u{947}\u{92e}\u{94d}\u{91f} 1861\",\"AM 11 \u{92e}\u{947}\u{917}\u{93e}\u{92c}\u{93f}\u{91f} 2023\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('hi-u-ca-persian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"AP 25 \u{921}\u{947} 1402, 12:00 am\",\"AP 11 \u{916}\u{94b}\u{930}\u{930}\u{94d}\u{926}\u{93e}\u{926} 1402, 12:00 am\",\"AP 10 \u{921}\u{947} 1369, 12:00 am\",\"AP 2 \u{905}\u{935}\u{928} 1247, 12:00 am\",\"AP 29 \u{908}\u{938}\u{94d}\u{92b}\u{928}\u{94d}\u{926}\u{94d} 1409, 12:00 am\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('hr-u-ca-indian', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"25. Pausa 1945. \u{15a}aka\",\"11. Jyaistha 1945. \u{15a}aka\",\"10. Pausa 1912. \u{15a}aka\",\"1. Kartika 1790. \u{15a}aka\",\"29. Phalguna 1952. \u{15a}aka\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('hu-u-ca-coptic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"AM 1740. T\u{fc}bi 6.\",\"AM 1739. Pakh\u{f3}nsz 24.\",\"AM 1707. Koiak 22.\",\"AM 1585. Paophi 14.\",\"AM 1747. Phamen\u{f3}th 11.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('hu-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"MF 1445. Red. 4. 0:00\",\"MF 1444. Ds\u{fc}l k. 13. 0:00\",\"MF 1411. Dsem. II 14. 0:00\",\"MF 1285. Red. 8. 0:00\",\"MF 1452. Ds\u{fc}l k. 27. 0:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('id-u-ca-ethiopic', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"6 Ter 2016 AM\",\"24 Genbot 2015 AM\",\"22 Tahsas 1983 AM\",\"14 Tekemt 1861 AM\",\"11 Megabit 2023 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('id-u-ca-roc', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"Senin, 15 Januari 113 ROC\",\"Kamis, 01 Juni 112 ROC\",\"Senin, 31 Desember 79 ROC\",\"Jumat, 23 Oktober 44 Sebelum R.O.C.\",\"Kamis, 20 Maret 120 ROC\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('it-u-ca-indian', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"25 Pausa\",\"11 Jyaistha\",\"10 Pausa\",\"1 Kartika\",\"29 Phalguna\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ja-u-ca-coptic', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"5\u{6708}6\u{65e5}\",\"9\u{6708}24\u{65e5}\",\"4\u{6708}22\u{65e5}\",\"2\u{6708}14\u{65e5}\",\"7\u{6708}11\u{65e5}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ja-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"AH1445/07/05 0:00\",\"AH1444/11/13 0:00\",\"AH1411/06/14 0:00\",\"AH1285/07/07 0:00\",\"AH1452/11/27 0:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('kk-u-ca-gregory', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"2024 M01 15\",\"2023 M06 1\",\"1990 M12 31\",\"1868 M10 23\",\"2031 M03 20\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('kn-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"BE 2567 \u{c9c}\u{ca8}\u{cb5}\u{cb0}\u{cbf} 15, \u{cb8}\u{ccb}\u{cae}\u{cb5}\u{cbe}\u{cb0}\",\"BE 2566 \u{c9c}\u{cc2}\u{ca8}\u{ccd} 1, \u{c97}\u{cc1}\u{cb0}\u{cc1}\u{cb5}\u{cbe}\u{cb0}\",\"BE 2533 \u{ca1}\u{cbf}\u{cb8}\u{cc6}\u{c82}\u{cac}\u{cb0}\u{ccd} 31, \u{cb8}\u{ccb}\u{cae}\u{cb5}\u{cbe}\u{cb0}\",\"BE 2411 \u{c85}\u{c95}\u{ccd}\u{c9f}\u{ccb}\u{cac}\u{cb0}\u{ccd} 23, \u{cb6}\u{cc1}\u{c95}\u{ccd}\u{cb0}\u{cb5}\u{cbe}\u{cb0}\",\"BE 2574 \u{cae}\u{cbe}\u{cb0}\u{ccd}\u{c9a}\u{ccd} 20, \u{c97}\u{cc1}\u{cb0}\u{cc1}\u{cb5}\u{cbe}\u{cb0}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('kn-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"4 \u{cb0}\u{c9c}\u{cac}\u{ccd}\",\"13 \u{ca7}\u{cc1}\u{2018}\u{cb2}\u{ccd}-\u{c95}\u{cbf}\u{2018}\u{ca1}\u{cbe}\u{cb9}\u{ccd}\",\"14 \u{c9c}\u{cc1}\u{cae}\u{cbe}\u{ca6}\u{cbe} II\",\"8 \u{cb0}\u{c9c}\u{cac}\u{ccd}\",\"27 \u{ca7}\u{cc1}\u{2018}\u{cb2}\u{ccd}-\u{c95}\u{cbf}\u{2018}\u{ca1}\u{cbe}\u{cb9}\u{ccd}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ko-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"40. 12. 5.\",\"40. 4. 13.\",\"7. 11. 15.\",\"5. 9. 8.\",\"48. 2. 27.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ko-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"AH 1445\u{b144} 7\u{c6d4} 3\u{c77c} \u{c6d4}\u{c694}\u{c77c}\",\"AH 1444\u{b144} 11\u{c6d4} 12\u{c77c} \u{baa9}\u{c694}\u{c77c}\",\"AH 1411\u{b144} 6\u{c6d4} 13\u{c77c} \u{c6d4}\u{c694}\u{c77c}\",\"AH 1285\u{b144} 7\u{c6d4} 6\u{c77c} \u{ae08}\u{c694}\u{c77c}\",\"AH 1452\u{b144} 11\u{c6d4} 26\u{c77c} \u{baa9}\u{c694}\u{c77c}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('kok-u-ca-gregory', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"2024-01\",\"2023-06\",\"1990-12\",\"1868-10\",\"2031-03\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('lt-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"2567 saus. 15 BE\",\"2566 bir\u{17e}. 1 BE\",\"2533 gruod. 31 BE\",\"2411 spal. 23 BE\",\"2574 kov. 20 BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('lt-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"1445-07-04 AH\",\"1444-11-12 AH\",\"1411-06-13 AH\",\"1285-07-06 AH\",\"1452-11-26 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('lv-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"2023 M12 5\",\"2023 M04 13\",\"1990 M11 15\",\"1868 M09 8\",\"2031 M02 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('lv-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"1445. gada 3. Raj. AH\",\"1444. gada 12. Dhu\u{2bb}l-Q. AH\",\"1411. gada 13. Jum. II AH\",\"1285. gada 6. Raj. AH\",\"1452. gada 26. Dhu\u{2bb}l-Q. AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ml-u-ca-gregory', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"2024 \u{d1c}\u{d28}\u{d41} 15, 12:00 AM\",\"2023 \u{d1c}\u{d42}\u{d7a} 1, 12:00 AM\",\"1990 \u{d21}\u{d3f}\u{d38}\u{d02} 31, 12:00 AM\",\"1868 \u{d12}\u{d15}\u{d4d}\u{d1f}\u{d4b} 23, 12:00 AM\",\"2031 \u{d2e}\u{d3e}\u{d7c} 20, 12:00 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('mr-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"\u{967}\u{96b} \u{91c}\u{93e}\u{928}\u{947}\u{935}\u{93e}\u{930}\u{940}, \u{968}\u{96b}\u{96c}\u{96d} \u{907}\u{938}\u{92a}\u{942}.\",\"\u{967} \u{91c}\u{942}\u{928}, \u{968}\u{96b}\u{96c}\u{96c} \u{907}\u{938}\u{92a}\u{942}.\",\"\u{969}\u{967} \u{921}\u{93f}\u{938}\u{947}\u{902}\u{92c}\u{930}, \u{968}\u{96b}\u{969}\u{969} \u{907}\u{938}\u{92a}\u{942}.\",\"\u{968}\u{969} \u{911}\u{915}\u{94d}\u{91f}\u{94b}\u{92c}\u{930}, \u{968}\u{96a}\u{967}\u{967} \u{907}\u{938}\u{92a}\u{942}.\",\"\u{968}\u{966} \u{92e}\u{93e}\u{930}\u{94d}\u{91a}, \u{968}\u{96b}\u{96d}\u{96a} \u{907}\u{938}\u{92a}\u{942}.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('mr-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"\u{96a} \u{930}\u{91d}\u{93e}\u{92c}, \u{967}\u{96a}\u{96a}\u{96b} \u{939}\u{93f}.\u{935}.\",\"\u{967}\u{968} \u{927}\u{941}\u{932}-\u{915}\u{940}\u{926}\u{93e}\u{939}, \u{967}\u{96a}\u{96a}\u{96a} \u{939}\u{93f}.\u{935}.\",\"\u{967}\u{969} \u{91c}\u{941}\u{92e}\u{93e}\u{926}\u{93e} II, \u{967}\u{96a}\u{967}\u{967} \u{939}\u{93f}.\u{935}.\",\"\u{96c} \u{930}\u{91d}\u{93e}\u{92c}, \u{967}\u{968}\u{96e}\u{96b} \u{939}\u{93f}.\u{935}.\",\"\u{968}\u{96c} \u{927}\u{941}\u{932}-\u{915}\u{940}\u{926}\u{93e}\u{939}, \u{967}\u{96a}\u{96b}\u{968} \u{939}\u{93f}.\u{935}.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ms-u-ca-dangi', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"Disember 5\",\"April 13\",\"November 15\",\"September 8\",\"Februari 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ms-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"3 Rejab 1445 H\",\"12 Zulkaedah 1444 H\",\"13 Jamadilakhir 1411 H\",\"6 Rejab 1285 H\",\"26 Zulkaedah 1452 H\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('nb-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"mandag 5. shevat 5784 AM\",\"torsdag 12. sivan 5783 AM\",\"mandag 14. tevet 5751 AM\",\"fredag 7. heshvan 5629 AM\",\"torsdag 25. adar 5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('nl-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"1-2567 BE\",\"6-2566 BE\",\"12-2533 BE\",\"10-2411 BE\",\"3-2574 BE\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('nl-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"04-07-45 AH\",\"12-11-44 AH\",\"13-06-11 AH\",\"06-07-85 AH\",\"26-11-52 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('no-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"6.5.7516 AA\",\"24.9.7515 AA\",\"22.4.7483 AA\",\"14.2.7361 AA\",\"11.7.7523 AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('no-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"7.1445 AH\",\"11.1444 AH\",\"6.1411 AH\",\"7.1285 AH\",\"11.1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('pa-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"AM 5784 Shevat 5\",\"AM 5783 Sivan 12\",\"AM 5751 Tevet 14\",\"AM 5629 Heshvan 7\",\"AM 5791 Adar 25\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('pl-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"15 sty 2567 e.b., 00:00\",\"1 cze 2566 e.b., 00:00\",\"31 gru 2533 e.b., 00:00\",\"23 pa\u{17a} 2411 e.b., 00:00\",\"20 mar 2574 e.b., 00:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('pl-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"4 Ra. 1445 AH\",\"12 Zu al-k. 1444 AH\",\"13 D\u{17c}u. II 1411 AH\",\"6 Ra. 1285 AH\",\"26 Zu al-k. 1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('pt-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"6 de Ter de 7516 AA\",\"24 de Genbot de 7515 AA\",\"22 de Tahsas de 7483 AA\",\"14 de Tekemt de 7361 AA\",\"11 de Megabit de 7523 AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('pt-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"3 de Raj. de 1445 AH, 00:00\",\"12 de Dhu\u{2bb}l-Q. de 1444 AH, 00:00\",\"13 de Jum. II de 1411 AH, 00:00\",\"6 de Raj. de 1285 AH, 00:00\",\"26 de Dhu\u{2bb}l-Q. de 1452 AH, 00:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ro-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"5 \u{218}evat 5784 A.M.\",\"12 Sivan 5783 A.M.\",\"14 Tevet 5751 A.M.\",\"7 He\u{219}van 5629 A.M.\",\"25 Adar 5791 A.M.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ru-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"\u{43f}\u{43e}\u{43d}\u{435}\u{434}\u{435}\u{43b}\u{44c}\u{43d}\u{438}\u{43a}, 5 M12 gui-mao\",\"\u{447}\u{435}\u{442}\u{432}\u{435}\u{440}\u{433}, 14 M04 gui-mao\",\"\u{43f}\u{43e}\u{43d}\u{435}\u{434}\u{435}\u{43b}\u{44c}\u{43d}\u{438}\u{43a}, 15 M11 geng-wu\",\"\u{43f}\u{44f}\u{442}\u{43d}\u{438}\u{446}\u{430}, 8 M09 wu-chen\",\"\u{447}\u{435}\u{442}\u{432}\u{435}\u{440}\u{433}, 28 M02 xin-hai\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ru-u-ca-islamic-civil', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"4 \u{440}\u{430}\u{434}\u{436}\u{430}\u{431}\",\"12 \u{437}\u{443}\u{43b}\u{44c}-\u{43a}\u{430}\u{430}\u{434}\u{430}\",\"13 \u{434}\u{436}\u{443}\u{43c}\u{430}\u{434}-\u{443}\u{43b}\u{44c}-\u{430}\u{445}\u{438}\u{440}\",\"6 \u{440}\u{430}\u{434}\u{436}\u{430}\u{431}\",\"26 \u{437}\u{443}\u{43b}\u{44c}-\u{43a}\u{430}\u{430}\u{434}\u{430}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sd-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"AA 7516-05-06\",\"AA 7515-09-24\",\"AA 7483-04-22\",\"AA 7361-02-14\",\"AA 7523-07-11\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sd-u-ca-persian', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"AP 1402-10-25\",\"AP 1402-03-11\",\"AP 1369-10-10\",\"AP 1247-08-02\",\"AP 1409-12-29\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sk-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"5/5784 AM\",\"9/5783 AM\",\"4/5751 AM\",\"2/5629 AM\",\"6/5791 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sl-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"2023 M12 5\",\"2023 M04 14\",\"1990 M11 15\",\"1868 M09 8\",\"2031 M02 28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sl-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"4. 7. 1445 AH\",\"13. 11. 1444 AH\",\"14. 6. 1411 AH\",\"8. 7. 1285 AH\",\"27. 11. 1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sr-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"6. \u{422}\u{435}\u{440} 7516. AA\",\"24. \u{413}\u{435}\u{43d}\u{431}\u{43e}\u{442} 7515. AA\",\"22. \u{422}\u{430}\u{445}\u{441}\u{430}\u{441} 7483. AA\",\"14. \u{422}\u{435}\u{43a}\u{435}\u{43c}\u{442} 7361. AA\",\"11. \u{41c}\u{435}\u{433}\u{430}\u{431}\u{438}\u{442} 7523. AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sr-u-ca-persian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"25. \u{414}\u{435}\u{458} 1402. AP\",\"11. \u{41a}\u{43e}\u{440}\u{434}\u{430}\u{434} 1402. AP\",\"10. \u{414}\u{435}\u{458} 1369. AP\",\"2. \u{410}\u{431}\u{430}\u{43d} 1247. AP\",\"29. \u{415}\u{441}\u{444}\u{430}\u{43d}\u{434} 1409. AP\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sv-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"5 shev\u{e1}t 5784 AM 00:00\",\"12 siv\u{e1}n 5783 AM 00:00\",\"14 tev\u{e9}t 5751 AM 00:00\",\"7 heshv\u{e1}n 5629 AM 00:00\",\"25 ad\u{e1}r 5791 AM 00:00\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sw-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"2023(gui-mao) M12 5\",\"2023(gui-mao) M04 14\",\"1990(geng-wu) M11 15\",\"1868(wu-chen) M09 8\",\"2031(xin-hai) M02 28\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('sw-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"4 Rajab 1445 AH\",\"13 Dhu\u{2bb}l-Qi\u{2bb}dah 1444 AH\",\"14 Jumada II 1411 AH\",\"8 Rajab 1285 AH\",\"27 Dhu\u{2bb}l-Qi\u{2bb}dah 1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ta-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"6 \u{ba4}\u{bc6}\u{bb0}\u{bcd}\",\"24 \u{b95}\u{bc6}\u{ba9}\u{bcd}\u{baa}\u{bcb}\",\"22 \u{ba4}\u{bb9}\u{bcd}\u{b9a}\u{bbe}\u{bb8}\u{bcd}\",\"14 \u{ba4}\u{bc6}\u{b95}\u{bc6}\u{bae}\u{bcd}\u{baa}\u{bcd}\u{ba4}\u{bcd}\",\"11 \u{bae}\u{b95}\u{bbe}\u{baa}\u{bbf}\u{b9f}\u{bcd}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ta-u-ca-persian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"25/10/1402 AP\",\"11/3/1402 AP\",\"10/10/1369 AP\",\"2/8/1247 AP\",\"29/12/1409 AP\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('te-u-ca-indian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'full'})).format(t)))",
            "[\"\u{c38}\u{c4b}\u{c2e}\u{c35}\u{c3e}\u{c30}\u{c02} 25 \u{c2a}\u{c41}\u{c37}\u{c4d}\u{c2f}\u{c02} 1945 \u{c36}\u{c15}\",\"\u{c17}\u{c41}\u{c30}\u{c41}\u{c35}\u{c3e}\u{c30}\u{c02} 11 \u{c1c}\u{c4d}\u{c2f}\u{c47}\u{c37}\u{c4d}\u{c20}\u{c02} 1945 \u{c36}\u{c15}\",\"\u{c38}\u{c4b}\u{c2e}\u{c35}\u{c3e}\u{c30}\u{c02} 10 \u{c2a}\u{c41}\u{c37}\u{c4d}\u{c2f}\u{c02} 1912 \u{c36}\u{c15}\",\"\u{c36}\u{c41}\u{c15}\u{c4d}\u{c30}\u{c35}\u{c3e}\u{c30}\u{c02} 1 \u{c15}\u{c3e}\u{c30}\u{c4d}\u{c24}\u{c40}\u{c15}\u{c02} 1790 \u{c36}\u{c15}\",\"\u{c17}\u{c41}\u{c30}\u{c41}\u{c35}\u{c3e}\u{c30}\u{c02} 29 \u{c2b}\u{c3e}\u{c32}\u{c4d}\u{c17}\u{c41}\u{c23}\u{c02} 1952 \u{c36}\u{c15}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('th-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"2023-12\",\"2023-04\",\"1990-11\",\"1868-09\",\"2031-02\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('th-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"4/7/1445 \u{e2e}.\u{e28}.\",\"13/11/1444 \u{e2e}.\u{e28}.\",\"14/6/1411 \u{e2e}.\u{e28}.\",\"8/7/1285 \u{e2e}.\u{e28}.\",\"27/11/1452 \u{e2e}.\u{e28}.\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('tr-u-ca-ethiopic', Object.assign({timeZone: 'UTC'}, {})).format(t)))",
            "[\"AM 06.05.2016\",\"AM 24.09.2015\",\"AM 22.04.1983\",\"AM 14.02.1861\",\"AM 11.07.2023\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('tr-u-ca-persian', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"AP 25 Dey 1402\",\"AP 11 Hordad 1402\",\"AP 10 Dey 1369\",\"AP 2 Aban 1247\",\"AP 29 Esfend 1409\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('uk-u-ca-indian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"25 \u{43f}\u{430}\u{443}\u{441}. 1945 \u{440}. \u{15a}aka\",\"11 \u{434}\u{436}\u{430}\u{439}. 1945 \u{440}. \u{15a}aka\",\"10 \u{43f}\u{430}\u{443}\u{441}. 1912 \u{440}. \u{15a}aka\",\"1 \u{43a}\u{430}\u{440}. 1790 \u{440}. \u{15a}aka\",\"29 \u{444}\u{430}\u{43b}\u{44c}. 1952 \u{440}. \u{15a}aka\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ur-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"2023 M12 5\u{60c} 12:00 AM\",\"2023 M04 14\u{60c} 12:00 AM\",\"1990 M11 15\u{60c} 12:00 AM\",\"1868 M09 8\u{60c} 12:00 AM\",\"2031 M02 28\u{60c} 12:00 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ur-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"4 \u{631}\u{62c}\u{628}\u{60c} 1445 \u{6c1}\u{62c}\u{631}\u{6cc}\",\"13 \u{630}\u{648}\u{627}\u{644}\u{642}\u{639}\u{62f}\u{6c3}\u{60c} 1444 \u{6c1}\u{62c}\u{631}\u{6cc}\",\"14 \u{62c}\u{645}\u{627}\u{62f}\u{6cc} \u{627}\u{644}\u{62b}\u{627}\u{646}\u{6cc}\u{60c} 1411 \u{6c1}\u{62c}\u{631}\u{6cc}\",\"8 \u{631}\u{62c}\u{628}\u{60c} 1285 \u{6c1}\u{62c}\u{631}\u{6cc}\",\"27 \u{630}\u{648}\u{627}\u{644}\u{642}\u{639}\u{62f}\u{6c3}\u{60c} 1452 \u{6c1}\u{62c}\u{631}\u{6cc}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('uz-u-ca-ethiopic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"AM 2016 Ter 6\",\"AM 2015 Genbot 24\",\"AM 1983 Tahsas 22\",\"AM 1861 Tekemt 14\",\"AM 2023 Megabit 11\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('uz-u-ca-persian', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"Dey 25\",\"Khordad 11\",\"Dey 10\",\"Aban 2\",\"Esfand 29\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('vi-u-ca-indian', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"25 Pausa, 1945 \u{15a}aka\",\"11 Jyaistha, 1945 \u{15a}aka\",\"10 Pausa, 1912 \u{15a}aka\",\"1 Kartika, 1790 \u{15a}aka\",\"29 Phalguna, 1952 \u{15a}aka\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('zh-u-ca-coptic', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"\u{79d1}\u{666e}\u{7279}\u{5386}1740\u{5e74}5\u{6708}6\u{65e5}\",\"\u{79d1}\u{666e}\u{7279}\u{5386}1739\u{5e74}9\u{6708}24\u{65e5}\",\"\u{79d1}\u{666e}\u{7279}\u{5386}1707\u{5e74}4\u{6708}22\u{65e5}\",\"\u{79d1}\u{666e}\u{7279}\u{5386}1585\u{5e74}2\u{6708}14\u{65e5}\",\"\u{79d1}\u{666e}\u{7279}\u{5386}1747\u{5e74}7\u{6708}11\u{65e5}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('zh-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"\u{4f0a}\u{65af}\u{5170}\u{5386}1445\u{5e74}7\u{6708}5\u{65e5}\",\"\u{4f0a}\u{65af}\u{5170}\u{5386}1444\u{5e74}11\u{6708}13\u{65e5}\",\"\u{4f0a}\u{65af}\u{5170}\u{5386}1411\u{5e74}6\u{6708}14\u{65e5}\",\"\u{4f0a}\u{65af}\u{5170}\u{5386}1285\u{5e74}7\u{6708}7\u{65e5}\",\"\u{4f0a}\u{65af}\u{5170}\u{5386}1452\u{5e74}11\u{6708}27\u{65e5}\"]",
        ),
    ]);
}

#[test]
fn calendar_ranges_and_parts_as_the_reference() {
    // Ranges, parts and options in other calendars (Chrome 156), a sample.
    check(&[
        (
            "JSON.stringify(new Intl.DateTimeFormat('en-u-ca-japanese', Object.assign({timeZone: 'UTC'}, {})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"1/15/6 \u{2013} 4/2/6 R\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('en-u-ca-hebrew', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'short'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"Shevat \u{2013} Adar II 5784\"",
        ),
        (
            "new Date(Date.UTC(2024, 6, 4, 15, 30)).toLocaleString('en-u-ca-buddhist', {timeZone: 'UTC'})",
            "7/4/2567 BE, 3:30:00 PM",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('de-u-ca-japanese', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"weekday\",\"value\":\"Montag\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"day\",\"value\":\"15\"},{\"type\":\"literal\",\"value\":\". \"},{\"type\":\"month\",\"value\":\"Januar\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"year\",\"value\":\"6\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"era\",\"value\":\"Reiwa\"}]",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('de-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"5. M12 gui-mao \u{2013} 24. M02 jia-chen\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('de-u-ca-persian', {timeZone: 'UTC', year: 'numeric', era: 'long'}).resolvedOptions())",
            "{\"locale\":\"de-u-ca-persian\",\"calendar\":\"persian\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"era\":\"long\",\"year\":\"numeric\"}",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ja-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"07/04\u{ff5e}09/23\"",
        ),
        (
            "new Date(Date.UTC(2024, 6, 4, 15, 30)).toLocaleString('ja-u-ca-roc', {timeZone: 'UTC'})",
            "\u{6c11}\u{56fd}113/7/4 15:30:00",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ja-u-ca-indian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"\u{30b5}\u{30ab}1945/10/25\u{ff5e}1946/01/13\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ar-u-ca-islamic-umalqura', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"weekday\",\"value\":\"\u{627}\u{644}\u{627}\u{62b}\u{646}\u{64a}\u{646}\"},{\"type\":\"literal\",\"value\":\"\u{60c} \"},{\"type\":\"day\",\"value\":\"3\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"month\",\"value\":\"\u{631}\u{62c}\u{628}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"year\",\"value\":\"1445\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"era\",\"value\":\"\u{647}\u{640}\"}]",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ar-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"15 \u{64a}\u{646}\u{627}\u{64a}\u{631} \u{2013} 2 \u{623}\u{628}\u{631}\u{64a}\u{644}\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fa-u-ca-japanese', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"\u{6f1}\u{6f5} \u{698}\u{627}\u{646}\u{648}\u{6cc}\u{647} \u{62a}\u{627} \u{6f2} \u{622}\u{648}\u{631}\u{6cc}\u{644} \u{6f6} \u{631}\u{6cc}\u{648}\u{627}\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fa-u-ca-hebrew', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"weekday\",\"value\":\"\u{62f}\u{648}\u{634}\u{646}\u{628}\u{647}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"day\",\"value\":\"\u{6f5}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"month\",\"value\":\"\u{634}\u{628}\u{627}\u{637}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"year\",\"value\":\"\u{6f5}\u{6f7}\u{6f8}\u{6f4}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"era\",\"value\":\"\u{62a}\u{642}\u{648}\u{6cc}\u{645} \u{639}\u{628}\u{631}\u{6cc}\"}]",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fa-u-ca-persian', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'short'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"\u{62f}\u{6cc} \u{6f1}\u{6f4}\u{6f0}\u{6f2} \u{62a}\u{627} \u{641}\u{631}\u{648}\u{631}\u{62f}\u{6cc}\u{646} \u{6f1}\u{6f4}\u{6f0}\u{6f3}\"",
        ),
        (
            "new Date(Date.UTC(2024, 6, 4, 15, 30)).toLocaleString('th-u-ca-japanese', {timeZone: 'UTC'})",
            "4/7/R 6 15:30:00",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('th-u-ca-chinese', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"weekday\",\"value\":\"\u{e27}\u{e31}\u{e19}\u{e08}\u{e31}\u{e19}\u{e17}\u{e23}\u{e4c}\"},{\"type\":\"literal\",\"value\":\", \"},{\"type\":\"yearName\",\"value\":\"\u{e01}\u{e38}\u{e4b}\u{e22}\u{e40}\u{e2b}\u{e21}\u{e48}\u{e32}\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"month\",\"value\":\"12\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"day\",\"value\":\"5\"}]",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('th-u-ca-ethiopic', Object.assign({timeZone: 'UTC'}, {})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"6/5/2016 \u{2013} 24/7/2016 AM\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('zh-u-ca-islamic', {timeZone: 'UTC', year: 'numeric', era: 'long'}).resolvedOptions())",
            "{\"locale\":\"zh-u-ca-islamic\",\"calendar\":\"islamic\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"era\":\"long\",\"year\":\"numeric\"}",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('zh-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"\u{4f5b}\u{5386}2567-01-15 \u{2013} 2567-04-02\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('zh-u-ca-indian', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"era\",\"value\":\"\u{5370}\u{5ea6}\u{5386}\"},{\"type\":\"year\",\"value\":\"1945\"},{\"type\":\"literal\",\"value\":\"\u{5e74}\"},{\"type\":\"month\",\"value\":\"10\"},{\"type\":\"literal\",\"value\":\"\u{6708}\"},{\"type\":\"day\",\"value\":\"25\"},{\"type\":\"literal\",\"value\":\"\u{65e5}\"},{\"type\":\"weekday\",\"value\":\"\u{661f}\u{671f}\u{4e00}\"}]",
        ),
        (
            "new Date(Date.UTC(2024, 6, 4, 15, 30)).toLocaleString('he-u-ca-islamic-umalqura', {timeZone: 'UTC'})",
            "28.12.1445 \u{5d4}\u{5d2}\u{5f3}\u{5e8}\u{5d4}, 15:30:00",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('he-u-ca-buddhist', {timeZone: 'UTC', year: 'numeric', era: 'long'}).resolvedOptions())",
            "{\"locale\":\"he-u-ca-buddhist\",\"calendar\":\"buddhist\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"era\":\"long\",\"year\":\"numeric\"}",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('hi-u-ca-japanese', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"15 \u{91c}\u{928}\u{935}\u{930}\u{940} \u{2013} 2 \u{905}\u{92a}\u{94d}\u{930}\u{948}\u{932}\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('hi-u-ca-chinese', Object.assign({timeZone: 'UTC'}, {})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"2023-12-05 \u{2013} 2024-02-24\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('hi-u-ca-persian', {timeZone: 'UTC', year: 'numeric', era: 'long'}).resolvedOptions())",
            "{\"locale\":\"hi-u-ca-persian\",\"calendar\":\"persian\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"era\":\"long\",\"year\":\"numeric\"}",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fr-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"4 rajab \u{2013} 23 ramadan\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fr-u-ca-roc', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'short'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"janv. \u{e0} avr.\u{a0}113\u{a0}RdC\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('fr-u-ca-ethiopic', {timeZone: 'UTC', dateStyle: 'full'}).formatToParts(Date.UTC(2024, 0, 15)))",
            "[{\"type\":\"weekday\",\"value\":\"lundi\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"day\",\"value\":\"6\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"month\",\"value\":\"ter\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"year\",\"value\":\"2016\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"era\",\"value\":\"AM\"}]",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ru-u-ca-islamic-umalqura', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"3 \u{440}\u{430}\u{434}\u{436}\u{430}\u{431} \u{2014} 23 \u{440}\u{430}\u{43c}\u{430}\u{434}\u{430}\u{43d}\"",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ru-u-ca-buddhist', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'short'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"\u{44f}\u{43d}\u{432}. 2567 \u{433}. BE \u{2014} \u{430}\u{43f}\u{440}. 2567 \u{433}. BE\"",
        ),
        (
            "new Date(Date.UTC(2024, 6, 4, 15, 30)).toLocaleString('ru-u-ca-indian', {timeZone: 'UTC'})",
            "13.04.1946 \u{421}\u{430}\u{43a}\u{430}, 15:30:00",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ko-u-ca-hebrew', {timeZone: 'UTC', year: 'numeric', era: 'long'}).resolvedOptions())",
            "{\"locale\":\"ko-u-ca-hebrew\",\"calendar\":\"hebrew\",\"numberingSystem\":\"latn\",\"timeZone\":\"UTC\",\"era\":\"long\",\"year\":\"numeric\"}",
        ),
        (
            "JSON.stringify(new Intl.DateTimeFormat('ko-u-ca-persian', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).formatRange(Date.UTC(2024, 0, 15), Date.UTC(2024, 3, 2)))",
            "\"AP 1402. 10. 25. ~ 1403. 1. 14.\"",
        ),
    ]);
}

#[test]
fn calendars_of_cycles_and_the_moon_as_the_reference() {
    // ICU's astronomical islamic calendar (ICU4X's simulated one is a day off) and the Chinese calendars' cyclic and related years (Chrome 156).
    check(&[
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ar-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"7\u{200f}/1445 \u{647}\u{640}\",\"11\u{200f}/1444 \u{647}\u{640}\",\"6\u{200f}/1411 \u{647}\u{640}\",\"7\u{200f}/1285 \u{647}\u{640}\",\"11\u{200f}/1452 \u{647}\u{640}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('ar-u-ca-islamic-rgsa', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"4 \u{631}\u{62c}\u{628}\",\"13 \u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}\u{629}\",\"14 \u{62c}\u{645}\u{627}\u{62f}\u{649} \u{627}\u{644}\u{622}\u{62e}\u{631}\u{629}\",\"8 \u{631}\u{62c}\u{628}\",\"27 \u{630}\u{648} \u{627}\u{644}\u{642}\u{639}\u{62f}\u{629}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('az-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {era: 'short', year: 'numeric', month: 'short', day: 'numeric'})).format(t)))",
            "[\"AH 1445 Raj. 5\",\"AH 1444 Dhu\u{2bb}l-Q. 13\",\"AH 1411 Jum. II 14\",\"AH 1285 Raj. 7\",\"AH 1452 Dhu\u{2bb}l-Q. 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('az-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"AH 1445-07\",\"AH 1444-11\",\"AH 1411-06\",\"AH 1285-07\",\"AH 1452-11\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bn-u-ca-indian', Object.assign({timeZone: 'UTC'}, {dateStyle: 'short'})).format(t)))",
            "[\"\u{9e8}\u{9eb}/\u{9e7}\u{9e6}/\u{9e7}\u{9ef}\u{9ea}\u{9eb} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7}\u{9e7}/\u{9e9}/\u{9e7}\u{9ef}\u{9ea}\u{9eb} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7}\u{9e6}/\u{9e7}\u{9e6}/\u{9e7}\u{9ef}\u{9e7}\u{9e8} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7}/\u{9ee}/\u{9e7}\u{9ed}\u{9ef}\u{9e6} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e8}\u{9ef}/\u{9e7}\u{9e8}/\u{9e7}\u{9ef}\u{9eb}\u{9e8} \u{9b8}\u{9be}\u{9b2}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('bn-u-ca-indian', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'long', day: 'numeric'})).format(t)))",
            "[\"\u{9e8}\u{9eb} \u{9aa}\u{9cc}\u{9b7}, \u{9e7}\u{9ef}\u{9ea}\u{9eb} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7}\u{9e7} \u{99c}\u{9cd}\u{9af}\u{9c8}\u{9b7}\u{9cd}\u{9a0}, \u{9e7}\u{9ef}\u{9ea}\u{9eb} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7}\u{9e6} \u{9aa}\u{9cc}\u{9b7}, \u{9e7}\u{9ef}\u{9e7}\u{9e8} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e7} \u{995}\u{9be}\u{9b0}\u{9cd}\u{9a4}\u{9bf}\u{995}, \u{9e7}\u{9ed}\u{9ef}\u{9e6} \u{9b8}\u{9be}\u{9b2}\",\"\u{9e8}\u{9ef} \u{9ab}\u{9be}\u{9b2}\u{9cd}\u{997}\u{9c1}\u{9a8}, \u{9e7}\u{9ef}\u{9eb}\u{9e8} \u{9b8}\u{9be}\u{9b2}\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('cs-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'long'})).format(t)))",
            "[\"4. red\u{17e}eb 1445 AH\",\"13. z\u{fa} l-ka\u{2019}da 1444 AH\",\"14. d\u{17e}um\u{e1}d\u{e1} al-\u{e1}chira 1411 AH\",\"8. red\u{17e}eb 1285 AH\",\"27. z\u{fa} l-ka\u{2019}da 1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('cs-u-ca-islamic', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium'})).format(t)))",
            "[\"4. 7. 1445 AH\",\"13. 11. 1444 AH\",\"14. 6. 1411 AH\",\"8. 7. 1285 AH\",\"27. 11. 1452 AH\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('en-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"Rajab 5\",\"Dhu\u{2bb}l-Qi\u{2bb}dah 13\",\"Jumada II 14\",\"Rajab 7\",\"Dhu\u{2bb}l-Qi\u{2bb}dah 27\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('en-u-ca-islamic-tbla', Object.assign({timeZone: 'UTC'}, {dateStyle: 'medium', timeStyle: 'short'})).format(t)))",
            "[\"Raj. 5, 1445 AH, 12:00 AM\",\"Dhu\u{2bb}l-Q. 13, 1444 AH, 12:00 AM\",\"Jum. II 14, 1411 AH, 12:00 AM\",\"Raj. 7, 1285 AH, 12:00 AM\",\"Dhu\u{2bb}l-Q. 27, 1452 AH, 12:00 AM\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('es-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {year: 'numeric', month: 'numeric'})).format(t)))",
            "[\"5/7516 AA\",\"9/7515 AA\",\"4/7483 AA\",\"2/7361 AA\",\"7/7523 AA\"]",
        ),
        (
            "JSON.stringify([Date.UTC(2024, 0, 15), Date.UTC(2023, 5, 1), Date.UTC(1990, 11, 31), Date.UTC(1868, 9, 23), Date.UTC(2031, 2, 20)].map(t => new Intl.DateTimeFormat('es-u-ca-ethioaa', Object.assign({timeZone: 'UTC'}, {month: 'long', day: 'numeric'})).format(t)))",
            "[\"6 de ter\",\"24 de genbot\",\"22 de tahsas\",\"14 de tekemt\",\"11 de megabit\"]",
        ),
    ]);
}
