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
            "Collator,DateTimeFormat,Locale,NumberFormat,PluralRules,getCanonicalLocales,supportedValuesOf",
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
