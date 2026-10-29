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
    for (source, expected) in cases {
        assert_eq!(evaluated(source), *expected, "{source}");
    }
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
            "Thursday, January 2, 2020 at 3:04:05\u{202f}PM UTC",
        ),
        (
            &format("dateStyle: 'medium', timeStyle: 'short'"),
            "Jan 2, 2020, 3:04\u{202f}PM",
        ),
        (&format("dateStyle: 'long'"), "January 2, 2020"),
        (&format("dateStyle: 'short'"), "1/2/20"),
        (&format("timeStyle: 'medium'"), "3:04:05\u{202f}PM"),
        (
            &format("timeStyle: 'full'"),
            "3:04:05\u{202f}PM Coordinated Universal Time",
        ),
        (
            &format("weekday: 'long', year: 'numeric', month: 'long', day: 'numeric'"),
            "Thursday, January 2, 2020",
        ),
        (
            &format("year: 'numeric', month: '2-digit', day: '2-digit'"),
            "01/02/2020",
        ),
        (
            &format("hour: '2-digit', minute: '2-digit'"),
            "03:04\u{202f}PM",
        ),
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
        (&format("hour: 'numeric'"), "3\u{202f}PM"),
        (
            &format!("new Date({at}).toLocaleString('en-US', {{timeZone: 'UTC'}})"),
            "1/2/2020, 3:04:05\u{202f}PM",
        ),
        (
            &format!("new Date({at}).toLocaleTimeString('en-US', {{timeZone: 'UTC'}})"),
            "3:04:05\u{202f}PM",
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
            "8:34\u{202f}PM",
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
            "1/2/2020, 3:04:05\u{202f}PM",
        ),
        (
            "new Date(2020, 6, 2, 9, 4, 5).toLocaleString('en-US')",
            "7/2/2020, 9:04:05\u{202f}AM",
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
            "[{\"type\":\"hour\",\"value\":\"3\"},{\"type\":\"literal\",\"value\":\":\"},{\"type\":\"minute\",\"value\":\"04\"},{\"type\":\"literal\",\"value\":\"\u{202f}\"},{\"type\":\"dayPeriod\",\"value\":\"PM\"}]",
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
            "RangeError: Incorrect locale information provided",
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
            "0.10000000000000000555",
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
