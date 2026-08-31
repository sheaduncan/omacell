//! Data-driven LCID rows. Adding a locale is a new table entry.

use super::{DateOrder, LocaleInfo, LocaleSeparators};

pub(crate) const TABLE: &[LocaleInfo] = &[
    EN_US, EN_GB, DE_DE, FR_FR, ES_ES, IT_IT, NL_NL, PT_BR, SV_SE, PL_PL, RU_RU, JA_JP, ZH_CN,
    KO_KR,
];

const EN_US: LocaleInfo = LocaleInfo {
    lcid: 0x0409,
    bcp47: "en-US",
    separators: LocaleSeparators::EN_US,
    date_order: DateOrder::Mdy,
    am: "AM",
    pm: "PM",
    months_full: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    months_abbr: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    days_full: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    days_abbr: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    currency: "$",
};

const EN_GB: LocaleInfo = LocaleInfo {
    lcid: 0x0809,
    bcp47: "en-GB",
    separators: LocaleSeparators::EN_US,
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: EN_US.months_full,
    months_abbr: EN_US.months_abbr,
    days_full: EN_US.days_full,
    days_abbr: EN_US.days_abbr,
    currency: "£",
};

const DE_DE: LocaleInfo = LocaleInfo {
    lcid: 0x0407,
    bcp47: "de-DE",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    months_abbr: [
        "Jan", "Feb", "Mrz", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ],
    days_full: [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ],
    days_abbr: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    currency: "€",
};

const FR_FR: LocaleInfo = LocaleInfo {
    lcid: 0x040C,
    bcp47: "fr-FR",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '\u{202F}',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ],
    months_abbr: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ],
    days_full: [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ],
    days_abbr: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    currency: "€",
};

const ES_ES: LocaleInfo = LocaleInfo {
    lcid: 0x040A,
    bcp47: "es-ES",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "a. m.",
    pm: "p. m.",
    months_full: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    months_abbr: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
    ],
    days_full: [
        "domingo",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
    ],
    days_abbr: ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"],
    currency: "€",
};

const IT_IT: LocaleInfo = LocaleInfo {
    lcid: 0x0410,
    bcp47: "it-IT",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    months_abbr: [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ],
    days_full: [
        "domenica",
        "lunedì",
        "martedì",
        "mercoledì",
        "giovedì",
        "venerdì",
        "sabato",
    ],
    days_abbr: ["dom", "lun", "mar", "mer", "gio", "ven", "sab"],
    currency: "€",
};

const NL_NL: LocaleInfo = LocaleInfo {
    lcid: 0x0413,
    bcp47: "nl-NL",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "januari",
        "februari",
        "maart",
        "april",
        "mei",
        "juni",
        "juli",
        "augustus",
        "september",
        "oktober",
        "november",
        "december",
    ],
    months_abbr: [
        "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
    ],
    days_full: [
        "zondag",
        "maandag",
        "dinsdag",
        "woensdag",
        "donderdag",
        "vrijdag",
        "zaterdag",
    ],
    days_abbr: ["zo", "ma", "di", "wo", "do", "vr", "za"],
    currency: "€",
};

const PT_BR: LocaleInfo = LocaleInfo {
    lcid: 0x0416,
    bcp47: "pt-BR",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: '.',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "janeiro",
        "fevereiro",
        "março",
        "abril",
        "maio",
        "junho",
        "julho",
        "agosto",
        "setembro",
        "outubro",
        "novembro",
        "dezembro",
    ],
    months_abbr: [
        "jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez",
    ],
    days_full: [
        "domingo",
        "segunda-feira",
        "terça-feira",
        "quarta-feira",
        "quinta-feira",
        "sexta-feira",
        "sábado",
    ],
    days_abbr: ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"],
    currency: "R$",
};

const SV_SE: LocaleInfo = LocaleInfo {
    lcid: 0x041D,
    bcp47: "sv-SE",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: ' ',
        list: ';',
    },
    date_order: DateOrder::Ymd,
    am: "AM",
    pm: "PM",
    months_full: [
        "januari",
        "februari",
        "mars",
        "april",
        "maj",
        "juni",
        "juli",
        "augusti",
        "september",
        "oktober",
        "november",
        "december",
    ],
    months_abbr: [
        "jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
    ],
    days_full: [
        "söndag", "måndag", "tisdag", "onsdag", "torsdag", "fredag", "lördag",
    ],
    days_abbr: ["sön", "mån", "tis", "ons", "tor", "fre", "lör"],
    currency: "kr",
};

const PL_PL: LocaleInfo = LocaleInfo {
    lcid: 0x0415,
    bcp47: "pl-PL",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: ' ',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "stycznia",
        "lutego",
        "marca",
        "kwietnia",
        "maja",
        "czerwca",
        "lipca",
        "sierpnia",
        "września",
        "października",
        "listopada",
        "grudnia",
    ],
    months_abbr: [
        "sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru",
    ],
    days_full: [
        "niedziela",
        "poniedziałek",
        "wtorek",
        "środa",
        "czwartek",
        "piątek",
        "sobota",
    ],
    days_abbr: ["niedz.", "pon.", "wt.", "śr.", "czw.", "pt.", "sob."],
    currency: "zł",
};

const RU_RU: LocaleInfo = LocaleInfo {
    lcid: 0x0419,
    bcp47: "ru-RU",
    separators: LocaleSeparators {
        decimal: ',',
        thousands: ' ',
        list: ';',
    },
    date_order: DateOrder::Dmy,
    am: "AM",
    pm: "PM",
    months_full: [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ],
    months_abbr: [
        "янв.",
        "февр.",
        "мар.",
        "апр.",
        "мая",
        "июн.",
        "июл.",
        "авг.",
        "сент.",
        "окт.",
        "нояб.",
        "дек.",
    ],
    days_full: [
        "воскресенье",
        "понедельник",
        "вторник",
        "среда",
        "четверг",
        "пятница",
        "суббота",
    ],
    days_abbr: ["вс", "пн", "вт", "ср", "чт", "пт", "сб"],
    currency: "₽",
};

const JA_JP: LocaleInfo = LocaleInfo {
    lcid: 0x0411,
    bcp47: "ja-JP",
    separators: LocaleSeparators::EN_US,
    date_order: DateOrder::Ymd,
    am: "午前",
    pm: "午後",
    months_full: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    months_abbr: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    days_full: [
        "日曜日",
        "月曜日",
        "火曜日",
        "水曜日",
        "木曜日",
        "金曜日",
        "土曜日",
    ],
    days_abbr: ["日", "月", "火", "水", "木", "金", "土"],
    currency: "¥",
};

const ZH_CN: LocaleInfo = LocaleInfo {
    lcid: 0x0804,
    bcp47: "zh-CN",
    separators: LocaleSeparators::EN_US,
    date_order: DateOrder::Ymd,
    am: "上午",
    pm: "下午",
    months_full: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    months_abbr: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    days_full: [
        "星期日",
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
    ],
    days_abbr: ["周日", "周一", "周二", "周三", "周四", "周五", "周六"],
    currency: "¥",
};

const KO_KR: LocaleInfo = LocaleInfo {
    lcid: 0x0412,
    bcp47: "ko-KR",
    separators: LocaleSeparators::EN_US,
    date_order: DateOrder::Ymd,
    am: "오전",
    pm: "오후",
    months_full: [
        "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
    ],
    months_abbr: [
        "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
    ],
    days_full: [
        "일요일",
        "월요일",
        "화요일",
        "수요일",
        "목요일",
        "금요일",
        "토요일",
    ],
    days_abbr: ["일", "월", "화", "수", "목", "금", "토"],
    currency: "₩",
};

pub(crate) fn lookup(lcid: u32) -> Option<&'static LocaleInfo> {
    TABLE.iter().find(|row| row.lcid == lcid)
}

pub(crate) fn lookup_bcp47(tag: &str) -> Option<&'static LocaleInfo> {
    TABLE.iter().find(|row| row.bcp47.eq_ignore_ascii_case(tag))
}
