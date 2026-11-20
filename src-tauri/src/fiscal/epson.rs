//! Epson "ePOS Fiscal Print" (fpmate.cgi) for the FP-81 II RT and FP-90 III RT: the XML of a request and the
//! reading of the answer. No I/O here.
//!
//! Every element and attribute comes from the "ePOS Fiscal Print Solution Development Guide", FP 000 055 EN
//! Rev. U (below: Guide), and the native error names from the "Communication Protocol" FP 000 008 EN Rev 8.10.
//! See `xmldocs/proposal/fiscal-epson.xml`.
use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};

/// Path and query of the request (Guide §2.2.2-2.2.3). Amounts in the answer use a dot.
pub const PATH: &str = "/cgi-bin/fpmate.cgi?devid=local_printer&timeout=10000&reply_decimal_separator=.";
/// `timeout` of the Z report, in ms: it answers after the transmission to the tax authority (Guide §3.4).
pub const Z_TIMEOUT_MS: u32 = 30_000;
/// Longest description on a commercial document (Guide §5.2).
const DESCRIPTION_MAX: usize = 38;
/// Characters the register cannot print (Guide §8), plus the no-break space.
const UNSUPPORTED: &str = "åÅæÆ¢¥₧ƒªº¿⌐¬¡αßΓπΣσµτΘΩδ∞ε∩≡±⌠⌡∙·√ⁿ²\u{a0}";
/// The operator of every request: the Guide assumes 1 when it is omitted (§5.2).
const OPERATOR: &str = "1";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentType {
    Cash,
    Cheque,
    Electronic,
    Ticket,
    NotPaid,
}

impl PaymentType {
    /// `paymentType` and the allowed range of `index` (Guide §5.4.14); `None`: `index` has no meaning.
    fn code(self) -> (u8, Option<(u8, u8)>) {
        match self {
            PaymentType::Cash => (0, Some((0, 5))),
            PaymentType::Cheque => (1, None),
            PaymentType::Electronic => (2, Some((1, 10))),
            PaymentType::Ticket => (3, Some((1, 10))),
            PaymentType::NotPaid => (5, Some((0, 5))),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Line {
    pub description: String,
    /// Decimal with a dot, up to 3 decimals, 0.001 to 9999.999.
    pub quantity: String,
    /// Decimal with a dot, up to 2 decimals, 0 to 9999999.99.
    pub unit_price: String,
    /// 1 to 99.
    pub department: u8,
}

#[derive(Debug, Deserialize)]
pub struct Payment {
    #[serde(rename = "type")]
    pub kind: PaymentType,
    /// Decimal with a dot, up to 2 decimals, 0 to 9999999.99.
    pub amount: String,
    /// The totaliser inside the type; the register uses 1 when it is missing.
    #[serde(default)]
    pub index: Option<u8>,
    /// Printed on the payment line.
    #[serde(default)]
    pub description: Option<String>,
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{body}</s:Body></s:Envelope>"
    )
}

fn check_text(what: &str, text: &str) -> Result<(), String> {
    if text.chars().count() > DESCRIPTION_MAX {
        return Err(format!("{what} «{text}» supera {DESCRIPTION_MAX} caratteri."));
    }
    if let Some(c) = text.chars().find(|c| c.is_control() || UNSUPPORTED.contains(*c)) {
        return Err(format!("{what} «{text}» contiene un carattere che il registratore non stampa: {c:?}."));
    }
    Ok(())
}

/// The value of a decimal with a dot and at most `decimals` decimals, in units of 10^-`decimals`.
fn decimal(what: &str, text: &str, decimals: u32) -> Result<u64, String> {
    let error = || format!("{what} «{text}» non è un numero con il punto e al massimo {decimals} decimali.");
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || whole.len() > 9 || !digits(whole) || fraction.len() > decimals as usize || !digits(fraction)
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(error());
    }
    let scale = 10u64.pow(decimals);
    let fraction_value = format!("{fraction:0<width$}", width = decimals as usize).parse::<u64>().unwrap_or(0);
    Ok(whole.parse::<u64>().map_err(|_| error())? * scale + fraction_value)
}

fn check_range(what: &str, text: &str, decimals: u32, min: u64, max: u64) -> Result<(), String> {
    let value = decimal(what, text, decimals)?;
    if value < min || value > max {
        return Err(format!("{what} «{text}» è fuori dall'intervallo ammesso dal registratore."));
    }
    Ok(())
}

/// `queryPrinterStatus` with `statusType` 0 (basic) or 1 (RT) (Guide §5.9.16).
pub fn status_request(rt: bool) -> String {
    let status_type = if rt { 1 } else { 0 };
    envelope(&format!(
        "<printerCommand><queryPrinterStatus operator=\"{OPERATOR}\" statusType=\"{status_type}\" /></printerCommand>"
    ))
}

/// A commercial document (Guide §3.2, §5.4): sales, then payments.
pub fn receipt_request(lines: &[Line], payments: &[Payment]) -> Result<String, String> {
    if lines.is_empty() {
        return Err("Lo scontrino non ha righe di vendita.".into());
    }
    if payments.is_empty() {
        return Err("Lo scontrino non ha pagamenti.".into());
    }
    let mut body = format!("<printerFiscalReceipt><beginFiscalReceipt operator=\"{OPERATOR}\" />");
    for line in lines {
        check_text("La descrizione", &line.description)?;
        check_range("La quantità", &line.quantity, 3, 1, 9_999_999)?;
        check_range("Il prezzo", &line.unit_price, 2, 0, 999_999_999)?;
        if !(1..=99).contains(&line.department) {
            return Err(format!("Il reparto {} non è fra 1 e 99.", line.department));
        }
        body.push_str(&format!(
            "<printRecItem operator=\"{OPERATOR}\" description=\"{}\" quantity=\"{}\" unitPrice=\"{}\" department=\"{}\" justification=\"1\" />",
            escape(&line.description),
            line.quantity,
            line.unit_price,
            line.department
        ));
    }
    for payment in payments {
        check_range("L'importo del pagamento", &payment.amount, 2, 0, 999_999_999)?;
        let (code, range) = payment.kind.code();
        let index = match (payment.index, range) {
            (None, _) => String::new(),
            (Some(index), Some((min, max))) if (min..=max).contains(&index) => format!(" index=\"{index}\""),
            (Some(index), _) => return Err(format!("L'indice {index} non vale per il pagamento {:?}.", payment.kind)),
        };
        let description = match &payment.description {
            Some(text) => {
                check_text("La descrizione del pagamento", text)?;
                format!(" description=\"{}\"", escape(text))
            }
            None => String::new(),
        };
        body.push_str(&format!(
            "<printRecTotal operator=\"{OPERATOR}\"{description} payment=\"{}\" paymentType=\"{code}\"{index} justification=\"2\" />",
            payment.amount
        ));
    }
    body.push_str(&format!("<endFiscalReceipt operator=\"{OPERATOR}\" /></printerFiscalReceipt>"));
    Ok(envelope(&body))
}

/// `printXReport` (financial report) or `printZReport` (daily closure) (Guide §5.8.2).
pub fn report_request(closure: bool) -> String {
    let command = if closure {
        format!("<printZReport operator=\"{OPERATOR}\" timeout=\"{Z_TIMEOUT_MS}\" />")
    } else {
        format!("<printXReport operator=\"{OPERATOR}\" />")
    };
    envelope(&format!("<printerFiscalReport>{command}</printerFiscalReport>"))
}

/// The `response` element of an answer (Guide §6.1).
#[derive(Debug, PartialEq)]
pub struct Answer {
    pub success: bool,
    pub code: String,
    pub status: String,
    /// The children of `addInfo`, except `elementList`.
    pub info: BTreeMap<String, String>,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn predefined(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// Reads the `response` element of the SOAP answer of fpmate.
pub fn parse_answer(xml: &str) -> Result<Answer, String> {
    let bad = |e: &dyn std::fmt::Display| format!("Risposta del registratore non leggibile: {e}");
    let mut reader = Reader::from_str(xml);
    let mut answer: Option<Answer> = None;
    let mut in_info = false;
    let mut field: Option<(String, String)> = None;
    loop {
        match reader.read_event().map_err(|e| bad(&e))? {
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == "response" => {
                let mut current = Answer { success: false, code: String::new(), status: String::new(), info: BTreeMap::new() };
                for attribute in e.attributes() {
                    let attribute = attribute.map_err(|e| bad(&e))?;
                    let value = attribute.normalized_value(XmlVersion::Implicit1_0).map_err(|e| bad(&e))?.trim().to_string();
                    match attribute.key.as_ref() {
                        "success" => current.success = value == "true" || value == "1",
                        "code" => current.code = value,
                        "status" => current.status = value,
                        _ => {}
                    }
                }
                answer = Some(current);
            }
            Event::Start(e) if local_name(e.name().as_ref()) == "addInfo" => in_info = true,
            Event::End(e) if local_name(e.name().as_ref()) == "addInfo" => in_info = false,
            Event::Start(e) if in_info => {
                field = Some((local_name(e.name().as_ref()).to_string(), String::new()));
            }
            Event::Empty(e) if in_info => {
                let name = local_name(e.name().as_ref()).to_string();
                if let Some(answer) = answer.as_mut() {
                    answer.info.insert(name, String::new());
                }
            }
            Event::Text(text) => {
                if let Some((_, value)) = field.as_mut() {
                    value.push_str(&text.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(reference) => {
                if let Some((_, value)) = field.as_mut() {
                    let c = match reference.resolve_char_ref().map_err(|e| bad(&e))? {
                        Some(c) => c,
                        None => {
                            let name: &str = &reference;
                            predefined(name).ok_or_else(|| bad(&format!("entità &{name};")))?
                        }
                    };
                    value.push(c);
                }
            }
            Event::End(_) if in_info => {
                if let (Some((name, value)), Some(answer)) = (field.take(), answer.as_mut()) {
                    if name != "elementList" {
                        answer.info.insert(name, value.trim().to_string());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    answer.ok_or_else(|| bad(&"manca l'elemento response"))
}

/// Name of a native error of the register (Communication Protocol §8.3.1).
fn native_error(status: &str) -> Option<&'static str> {
    Some(match status.parse::<u8>().ok()? {
        2 => "CARTA SCONTRINO",
        3 => "OFFLINE",
        7 => "SLIP KO",
        8 => "TASTO ERRATO",
        9 => "DATA INFERIORE",
        10 => "DATA ERRATA",
        11 => "SEQUENZA ERRATA",
        12 => "DATI INESISTENTI",
        13 => "VALORE ERRATO",
        14 => "PROG MATRICOLA",
        15 => "GIA ESISTENTE",
        16 => "NON PREVISTO",
        17 => "IMPOSSIBILE ORA",
        18 => "NON POSSIBILE",
        19 => "SCRITTA INVALIDA",
        20 => "SUPERA VALORE",
        21 => "SUPERA LIMITE",
        22 => "NON PROGRAMMATO",
        23 => "CHIUDI SCONTRINO",
        24 => "CHIUDI PAGAMENTO",
        25 => "MANCA OPERATORE",
        26 => "CASSA INFERIORE",
        27 => "OLTRE PROGRAMMAZIONE",
        28 => "P.C. NON CONNESSO",
        29 => "MANCA MODULO",
        30 => "CHECKSUM ERRATO",
        34 => "MANCA ATTIVAZIONE",
        35 => "SLIP: CONNESSIONE ?",
        37 => "RIMUOVERE MODULO",
        38 => "EFT-POS IN ERRORE",
        39 => "DOC GIA ANNULLATO",
        40 => "DOC GIA RESO",
        41 => "TIPO NON VALIDO (DOC DI ANNULLO)",
        42 => "TIPO NON VALIDO (DOC DI RESO)",
        _ => return None,
    })
}

impl Answer {
    /// The answer itself when `success` is true; otherwise an error with `code`, `status` and the native name.
    pub fn checked(self) -> Result<Answer, String> {
        if self.success {
            return Ok(self);
        }
        // The Guide writes the same code with spaces or underscores (§6.2-6.7).
        let normalized: String = self.code.chars().filter(|c| *c != ' ' && *c != '_').collect::<String>().to_uppercase();
        let detail = match native_error(&self.status) {
            Some(name) if normalized == "PRINTERERROR" => format!(" ({name})"),
            _ => String::new(),
        };
        Err(format!("Il registratore risponde {} {}{detail}.", self.code, self.status))
    }

    fn take(&mut self, name: &str) -> Result<String, String> {
        self.info.remove(name).ok_or_else(|| format!("La risposta del registratore non ha {name}."))
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Receipt {
    pub number: String,
    pub amount: String,
    pub date: String,
    pub time: String,
    pub z_number: String,
    /// From fpmate 1.042 (Guide §5.4.1).
    pub serial: Option<String>,
}

pub fn receipt(answer: Answer) -> Result<Receipt, String> {
    let mut answer = answer.checked()?;
    Ok(Receipt {
        number: answer.take("fiscalReceiptNumber")?,
        amount: answer.take("fiscalReceiptAmount")?,
        date: answer.take("fiscalReceiptDate")?,
        time: answer.take("fiscalReceiptTime")?,
        z_number: answer.take("zRepNumber")?,
        serial: answer.info.remove("serialNumber"),
    })
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Report {
    pub z_number: String,
    pub daily_amount: String,
    pub serial: Option<String>,
}

pub fn report(answer: Answer) -> Result<Report, String> {
    let mut answer = answer.checked()?;
    Ok(Report {
        z_number: answer.take("zRepNumber")?,
        daily_amount: answer.take("dailyAmount")?,
        serial: answer.info.remove("serialNumber"),
    })
}

/// The two status answers (Guide §5.9.16), with the field names of the Guide.
#[derive(Debug, PartialEq, Serialize)]
pub struct Status {
    /// Five characters, Communication Protocol 1-074: printer, journal, drawer, document, operative state.
    pub fp_status: String,
    pub cpu_rel: String,
    pub mf_rel: String,
    pub mf_status: String,
    /// `rtType`, `rtMainStatus`, `rtSubStatus`, `rtDailyOpen`, … as the register returns them.
    pub rt: BTreeMap<String, String>,
}

pub fn status(basic: Answer, rt: Answer) -> Result<Status, String> {
    let mut basic = basic.checked()?;
    let mut rt = rt.checked()?;
    rt.info.remove("lastCommand");
    Ok(Status {
        fp_status: basic.take("fpStatus")?,
        cpu_rel: basic.take("cpuRel")?,
        mf_rel: basic.take("mfRel")?,
        mf_status: basic.take("mfStatus")?,
        rt: rt.info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>";
    const SUFFIX: &str = "</s:Body></s:Envelope>";

    fn wrap(body: &str) -> String {
        format!("{PREFIX}{body}{SUFFIX}")
    }

    fn line(description: &str, quantity: &str, unit_price: &str) -> Line {
        Line { description: description.into(), quantity: quantity.into(), unit_price: unit_price.into(), department: 1 }
    }

    fn cash(amount: &str) -> Payment {
        Payment { kind: PaymentType::Cash, amount: amount.into(), index: None, description: None }
    }

    #[test]
    fn status_requests() {
        assert_eq!(
            status_request(false),
            wrap("<printerCommand><queryPrinterStatus operator=\"1\" statusType=\"0\" /></printerCommand>")
        );
        assert_eq!(
            status_request(true),
            wrap("<printerCommand><queryPrinterStatus operator=\"1\" statusType=\"1\" /></printerCommand>")
        );
    }

    #[test]
    fn report_requests() {
        assert_eq!(report_request(false), wrap("<printerFiscalReport><printXReport operator=\"1\" /></printerFiscalReport>"));
        assert_eq!(
            report_request(true),
            wrap("<printerFiscalReport><printZReport operator=\"1\" timeout=\"30000\" /></printerFiscalReport>")
        );
    }

    #[test]
    fn receipt_request_with_two_payments() {
        let lines = [line("PANINO & BIBITA", "2", "6.00")];
        let payments = [
            cash("10.00"),
            Payment { kind: PaymentType::Electronic, amount: "2.00".into(), index: Some(1), description: Some("BANCOMAT".into()) },
        ];
        assert_eq!(
            receipt_request(&lines, &payments).unwrap(),
            wrap(concat!(
                "<printerFiscalReceipt><beginFiscalReceipt operator=\"1\" />",
                "<printRecItem operator=\"1\" description=\"PANINO &amp; BIBITA\" quantity=\"2\" unitPrice=\"6.00\" department=\"1\" justification=\"1\" />",
                "<printRecTotal operator=\"1\" payment=\"10.00\" paymentType=\"0\" justification=\"2\" />",
                "<printRecTotal operator=\"1\" description=\"BANCOMAT\" payment=\"2.00\" paymentType=\"2\" index=\"1\" justification=\"2\" />",
                "<endFiscalReceipt operator=\"1\" /></printerFiscalReceipt>"
            ))
        );
    }

    #[test]
    fn receipt_request_is_refused_for_bad_input() {
        let ok = [cash("1.00")];
        let refused = |lines: &[Line], payments: &[Payment]| receipt_request(lines, payments).unwrap_err();
        assert!(refused(&[], &ok).contains("righe"));
        assert!(refused(&[line("A", "1", "1.00")], &[]).contains("pagamenti"));
        assert!(refused(&[line(&"X".repeat(39), "1", "1.00")], &ok).contains("38"));
        assert!(refused(&[line("CAFFÈ ± LATTE", "1", "1.00")], &ok).contains("non stampa"));
        assert!(refused(&[line("A", "1", "1,00")], &ok).contains("punto"));
        assert!(refused(&[line("A", "1", "1.005")], &ok).contains("2 decimali"));
        assert!(refused(&[line("A", "1.", "1.00")], &ok).contains("punto"));
        assert!(refused(&[line("A", "0", "1.00")], &ok).contains("intervallo"));
        assert!(refused(&[line("A", "10000", "1.00")], &ok).contains("intervallo"));
        assert!(refused(&[line("A", "1", "10000000.00")], &ok).contains("intervallo"));
        let mut department = line("A", "1", "1.00");
        department.department = 0;
        assert!(refused(&[department], &ok).contains("reparto"));
        let cheque = Payment { kind: PaymentType::Cheque, amount: "1.00".into(), index: Some(1), description: None };
        assert!(refused(&[line("A", "1", "1.00")], &[cheque]).contains("indice"));
        let electronic = Payment { kind: PaymentType::Electronic, amount: "1.00".into(), index: Some(11), description: None };
        assert!(refused(&[line("A", "1", "1.00")], &[electronic]).contains("indice"));
    }

    #[test]
    fn a_float_from_javascript_is_refused() {
        let payment: Result<Payment, _> = serde_json::from_str(r#"{"type": "cash", "amount": 1.5}"#);
        assert!(payment.is_err());
        let payment: Payment = serde_json::from_str(r#"{"type": "not_paid", "amount": "1.50", "index": 2}"#).unwrap();
        assert_eq!(payment.kind, PaymentType::NotPaid);
    }

    #[test]
    fn receipt_answer() {
        let xml = wrap(concat!(
            "<response success=\"true\" code=\"\" status=\"2\"><addInfo>",
            "<elementList>lastCommand,printerStatus,fiscalReceiptNumber,fiscalReceiptAmount,fiscalReceiptDate,fiscalReceiptTime,zRepNumber,serialNumber</elementList>",
            "<lastCommand>74</lastCommand><printerStatus>00010</printerStatus>",
            "<fiscalReceiptNumber>2</fiscalReceiptNumber><fiscalReceiptAmount>12.00</fiscalReceiptAmount>",
            "<fiscalReceiptDate>21/04/2023</fiscalReceiptDate><fiscalReceiptTime>11:35</fiscalReceiptTime>",
            "<zRepNumber>39</zRepNumber><serialNumber>99IEB004001</serialNumber>",
            "</addInfo></response>"
        ));
        assert_eq!(
            receipt(parse_answer(&xml).unwrap()).unwrap(),
            Receipt {
                number: "2".into(),
                amount: "12.00".into(),
                date: "21/04/2023".into(),
                time: "11:35".into(),
                z_number: "39".into(),
                serial: Some("99IEB004001".into()),
            }
        );
    }

    #[test]
    fn report_answer_without_serial() {
        let xml = wrap(concat!(
            "<response success=\"true\" code=\"\" status=\"2\"><addInfo>",
            "<elementList>lastCommand,printerStatus,zRepNumber,dailyAmount</elementList>",
            "<lastCommand>74</lastCommand><printerStatus>20110</printerStatus>",
            "<zRepNumber>11</zRepNumber><dailyAmount>332.33</dailyAmount>",
            "</addInfo></response>"
        ));
        assert_eq!(
            report(parse_answer(&xml).unwrap()).unwrap(),
            Report { z_number: "11".into(), daily_amount: "332.33".into(), serial: None }
        );
    }

    #[test]
    fn status_answers() {
        let basic = wrap(concat!(
            "<response success=\"true\" code=\"\" status=\"2\"><addInfo>",
            "<elementList>lastCommand,cpuRel,mfRel,mfStatus,fpStatus</elementList>",
            "<lastCommand>74</lastCommand><cpuRel>07.00</cpuRel><mfRel>04.3</mfRel><mfStatus>0</mfStatus><fpStatus>00110</fpStatus>",
            "</addInfo></response>"
        ));
        let rt = wrap(concat!(
            "<response success=\"true\" code=\"\" status=\"2\"><addInfo>",
            "<elementList>lastCommand,rtType,rtMainStatus,rtSubStatus,rtDailyOpen</elementList>",
            "<lastCommand>138</lastCommand><rtType>I</rtType><rtMainStatus>02</rtMainStatus>",
            "<rtSubStatus>08</rtSubStatus><rtDailyOpen>1</rtDailyOpen>",
            "</addInfo></response>"
        ));
        let status = status(parse_answer(&basic).unwrap(), parse_answer(&rt).unwrap()).unwrap();
        assert_eq!(status.fp_status, "00110");
        assert_eq!(status.cpu_rel, "07.00");
        assert_eq!(status.mf_rel, "04.3");
        assert_eq!(status.mf_status, "0");
        assert_eq!(
            status.rt.into_iter().collect::<Vec<_>>(),
            [("rtDailyOpen", "1"), ("rtMainStatus", "02"), ("rtSubStatus", "08"), ("rtType", "I")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
        );
    }

    #[test]
    fn printer_error_carries_the_native_name() {
        let xml = wrap(concat!(
            "<response success=\"false\" code=\"PRINTER ERROR\" status=\"21\"><addInfo>",
            "<elementList>lastCommand,printerStatus</elementList>",
            "<lastCommand>80</lastCommand><printerStatus>00100</printerStatus>",
            "</addInfo></response>"
        ));
        assert_eq!(receipt(parse_answer(&xml).unwrap()).unwrap_err(), "Il registratore risponde PRINTER ERROR 21 (SUPERA LIMITE).");
    }

    #[test]
    fn offline_answer_without_element_list() {
        let xml = wrap(concat!(
            "<response success=\"false\" code=\"EPTR_REC_EMPTY\" status=\"3\"><addInfo>",
            "<lastCommand>74</lastCommand><printerStatus>30110</printerStatus>",
            "</addInfo></response>"
        ));
        let answer = parse_answer(&xml).unwrap();
        assert_eq!(answer.info.get("printerStatus").map(String::as_str), Some("30110"));
        assert_eq!(answer.checked().unwrap_err(), "Il registratore risponde EPTR_REC_EMPTY 3.");
    }

    #[test]
    fn code_with_a_leading_space_and_an_empty_response() {
        let answer = parse_answer(&wrap("<response success=\"false\" code=\" INCOMPLETE FILE\" status=\"0\" />")).unwrap();
        assert_eq!(answer.code, "INCOMPLETE FILE");
        assert!(answer.info.is_empty());
    }

    #[test]
    fn entities_in_a_value() {
        let xml = wrap("<response success=\"true\" code=\"\" status=\"2\"><addInfo><x>A&amp;B&#33;</x></addInfo></response>");
        assert_eq!(parse_answer(&xml).unwrap().info.get("x").map(String::as_str), Some("A&B!"));
    }

    #[test]
    fn not_an_answer() {
        assert!(parse_answer("<html><body>404</body></html>").unwrap_err().contains("response"));
        assert!(parse_answer("<response success=\"true\"><addInfo></response>").is_err());
    }
}
