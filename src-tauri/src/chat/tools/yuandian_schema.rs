//! 元典内置工具的集中 JSON Schema。
//!
//! 工具名和 local-first 执行逻辑仍在各业务模块；这里只维护模型可填写的官方参数、
//! 枚举和边界，避免法律、案例、企业工具各自手写后再次漂移。

use serde_json::{json, Value};

const EFFECT_LEVELS: &[&str] = &[
    "宪法",
    "法律",
    "司法解释",
    "行政法规",
    "监察法规",
    "部门规章",
    "党内法规",
    "军事法规规章",
    "立法机关工作文件",
    "行政机关工作文件",
    "行业/团体规范",
    "地方性法规",
    "自治条例和单行条例",
    "地方司法文件",
    "地方政府规章",
    "地方规范性文件",
    "地方律协规定",
];

const VALIDITIES: &[&str] = &["现行有效", "失效", "已被修改", "部分失效", "尚未生效"];

const REGIONS: &[&str] = &[
    "中央",
    "北京",
    "天津",
    "河北",
    "山西",
    "内蒙古",
    "辽宁",
    "吉林",
    "黑龙江",
    "上海",
    "江苏",
    "浙江",
    "安徽",
    "福建",
    "江西",
    "山东",
    "河南",
    "湖北",
    "湖南",
    "广东",
    "广西",
    "海南",
    "重庆",
    "四川",
    "贵州",
    "云南",
    "西藏",
    "陕西",
    "甘肃",
    "青海",
    "宁夏",
    "新疆",
];

const CASE_REGIONS: &[&str] = &[
    "北京",
    "天津",
    "河北",
    "山西",
    "内蒙古",
    "辽宁",
    "吉林",
    "黑龙江",
    "上海",
    "江苏",
    "浙江",
    "安徽",
    "福建",
    "江西",
    "山东",
    "河南",
    "湖北",
    "湖南",
    "广东",
    "广西",
    "海南",
    "重庆",
    "四川",
    "贵州",
    "云南",
    "西藏",
    "陕西",
    "甘肃",
    "青海",
    "宁夏",
    "新疆",
    "最高",
    "新疆生产建设兵团",
];

const CASE_CATEGORIES: &[&str] = &[
    "刑事案件",
    "民事案件",
    "行政案件",
    "执行案件",
    "管辖案件",
    "国家赔偿与司法救助案件",
    "强制清算与破产案件",
    "国际司法协助案件",
    "非诉保全审查案件",
    "其他案件",
];

const AUTHORITY_SOURCES: &[&str] = &[
    "典型案例",
    "参考案例",
    "公报案例",
    "解纷案例",
    "参阅案例",
    "刑事参考案例",
    "指导性案例",
    "检指导案例",
];

// 上市公司公告(rh_ssgsgg_search)专用枚举 —— 跟 REGIONS/CASE_REGIONS 都不同:
// 没有"中央/最高",但多出"境外""香港"。照官方取值范围原样登记,别复用上面两个。
const LISTED_MARKETS: &[&str] = &["深证A股", "上证A股", "北证A股"];

const LISTED_AREAS: &[&str] = &[
    "浙江",
    "北京",
    "广东",
    "江苏",
    "上海",
    "山东",
    "四川",
    "安徽",
    "福建",
    "湖北",
    "湖南",
    "河南",
    "重庆",
    "辽宁",
    "江西",
    "河北",
    "新疆",
    "陕西",
    "海南",
    "天津",
    "甘肃",
    "云南",
    "吉林",
    "黑龙江",
    "广西",
    "山西",
    "贵州",
    "西藏",
    "宁夏",
    "内蒙古",
    "青海",
    "境外",
    "香港",
];

fn date(description: &str) -> Value {
    json!({"type": "string", "format": "date", "description": description})
}

fn string_array(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

fn enum_array(values: &[&str], description: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "string", "enum": values},
        "uniqueItems": true,
        "description": description
    })
}

pub(super) fn law_keyword_search(keyword_required: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "keyword": {"type": "string", "description": "法规/法条正文关键词；多个词用空格分隔"},
            "search_mode": {"type": "string", "enum": ["AND", "OR"], "default": "AND", "description": "空格分词后的组合方式"},
            "fgmc": {"type": "string", "description": "法规名称过滤，标题需命中全部空格分词"},
            "xljb_1": {"type": "string", "enum": EFFECT_LEVELS, "description": "效力级别"},
            "sxx": {"type": "string", "enum": VALIDITIES, "description": "时效性"},
            "dy": {"type": "string", "enum": REGIONS, "description": "地域过滤"},
            "fbbm": {"type": "string", "description": "发布部门过滤"},
            "fbrq_start": date("发布日期起，含当日，YYYY-MM-DD"),
            "fbrq_end": date("发布日期止，含当日，YYYY-MM-DD"),
            "ssrq_start": date("实施日期起，含当日，YYYY-MM-DD"),
            "ssrq_end": date("实施日期止，含当日，YYYY-MM-DD"),
            "top_k": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20, "description": "返回候选数；接口最多 50"}
        }
    });
    if keyword_required {
        schema["required"] = json!(["keyword"]);
    }
    schema
}

pub(super) fn law_article_detail() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string", "description": "元典法条 ID，优先使用"},
            "fgmc": {"type": "string", "description": "法规全名，与 ftnum 同时使用"},
            "ftnum": {"type": "string", "description": "条号/条名，与 fgmc 或 fgid 同时使用"},
            "fgid": {"type": "string", "description": "CaseBoard 扩展：元典法规版本 ID，与 ftnum 使用可整部缓存后本地抽条"},
            "refer_date": date("历史时点版本参考日期，YYYY-MM-DD")
        }
    })
}

pub(super) fn regulation_detail() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string", "description": "元典法规 ID，优先使用"},
            "fgmc": {"type": "string", "description": "法规全名"},
            "refer_date": date("历史时点版本参考日期，YYYY-MM-DD")
        }
    })
}

pub(super) fn law_vector_search() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {"type": "string", "description": "待检索的自然语言法律问题"},
            "rewrite_flag": {"type": "boolean", "default": true, "description": "是否由元典改写检索问题"},
            "fatiao_filter": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "sxx": enum_array(VALIDITIES, "时效性，多值为或关系"),
                    "effect1": enum_array(EFFECT_LEVELS, "效力级别，多值为或关系"),
                    "law_start": date("实施日期起，含当日，YYYY-MM-DD"),
                    "law_end": date("实施日期止，含当日，YYYY-MM-DD")
                }
            },
            "return_num": {"type": "integer", "minimum": 1, "maximum": 50, "default": 45, "description": "返回法条数量，官方默认 45"}
        },
        "required": ["query"]
    })
}

pub(super) fn case_keyword_search(authority: bool) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "ah".into(),
        json!({"type": "string", "description": "完整案号"}),
    );
    properties.insert(
        "title".into(),
        json!({"type": "string", "description": "标题关键词，空格分词全部命中"}),
    );
    properties.insert("ay".into(), string_array("案由数组，多值为或关系"));
    properties.insert("jbdw".into(), string_array("法院/承办单位完整名称数组"));
    if authority {
        properties.insert(
            "source".into(),
            enum_array(AUTHORITY_SOURCES, "权威案例来源"),
        );
    } else {
        properties.insert(
            "ssqy".into(),
            json!({"type": "string", "description": "涉诉企业名称子串"}),
        );
        properties.insert(
            "fxgc".into(),
            json!({"type": "string", "description": "裁判分析过程关键词"}),
        );
        properties.insert(
            "yyft".into(),
            string_array("援引法条数组，每项写完整法规名和中文条号"),
        );
        properties.insert("ft_search_mode".into(), json!({"type": "string", "enum": ["and", "or"], "default": "and", "description": "援引法条组合方式"}));
    }
    properties.insert("xzqh_p".into(), enum_array(CASE_REGIONS, "省级行政区"));
    properties.insert(
        "wszl".into(),
        enum_array(&["判决书", "裁定书", "调解书", "决定书"], "文书种类"),
    );
    properties.insert(
        "ajlb".into(),
        json!({"type": "string", "enum": CASE_CATEGORIES, "description": "案件类别"}),
    );
    properties.insert("ja_start".into(), date("裁判/结案日期起，YYYY-MM-DD"));
    properties.insert("ja_end".into(), date("裁判/结案日期止，YYYY-MM-DD"));
    properties.insert(
        "qw".into(),
        json!({"type": "string", "description": "全文关键词，多个词用空格分隔"}),
    );
    properties.insert("search_mode".into(), json!({"type": "string", "enum": ["and", "or"], "default": "and", "description": "全文/分析关键词组合方式"}));
    properties.insert("top_k".into(), json!({"type": "integer", "minimum": 1, "maximum": 50, "default": 20, "description": "返回案例数量"}));
    json!({"type": "object", "additionalProperties": false, "properties": properties})
}

pub(super) fn case_detail() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string", "description": "案例 ID，优先使用"},
            "ah": {"type": "string", "description": "完整案号；未传 id 时使用"},
            "type": {"type": "string", "enum": ["ptal", "qwal"], "description": "可选：普通案例或权威案例库"}
        }
    })
}

pub(super) fn case_vector_search() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {"type": "string", "description": "待检索的自然语言裁判问题"},
            "rewrite_flag": {"type": "boolean", "default": true, "description": "是否由元典改写检索问题"},
            "wenshu_filter": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "wenshu_type": {"type": "string", "enum": CASE_CATEGORIES, "description": "案件类别"},
                    "ay": string_array("案由数组，多值为或关系"),
                    "wszl": {"type": "array", "items": {"type": "string", "enum": ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"]}, "description": "文书种类编码数组"},
                    "ja_start": date("结案日期起，YYYY-MM-DD"),
                    "ja_end": date("结案日期止，YYYY-MM-DD"),
                    "dianxing": {"type": "boolean", "default": false, "description": "true 时仅权威案例"},
                    "fayuan": string_array("法院完整名称数组"),
                    "source": enum_array(AUTHORITY_SOURCES, "权威案例来源，仅权威案例生效"),
                    "cj": {"type": "string", "enum": ["基层", "中级", "高级", "最高"], "description": "法院层级"},
                    "xzqh_p": {"type": "string", "enum": CASE_REGIONS, "description": "省级行政区"},
                    "xzqh_c": {"type": "string", "description": "地级市完整名称"}
                }
            },
            "return_num": {"type": "integer", "minimum": 1, "maximum": 50, "default": 45, "description": "返回案例数量，官方默认 45"}
        },
        "required": ["query"]
    })
}

fn entity_properties() -> Value {
    json!({
        "id": {"type": "string", "description": "元典企业 ID，优先使用"},
        "tyshxydm": {"type": "string", "minLength": 18, "maxLength": 18, "description": "统一社会信用代码"}
    })
}

pub(super) fn enterprise_search() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": {"type": "string", "description": "企业全称、简称或名称关键词"},
            "top_k": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10, "description": "返回企业候选数量"}
        },
        "required": ["name"]
    })
}

pub(super) fn enterprise_entity() -> Value {
    json!({"type": "object", "additionalProperties": false, "properties": entity_properties()})
}

pub(super) fn enterprise_paged() -> Value {
    let mut properties = entity_properties().as_object().cloned().unwrap_or_default();
    properties.insert("page".into(), json!({"type": "integer", "minimum": 1, "default": 1, "description": "页码，对应官方 pageNo"}));
    json!({"type": "object", "additionalProperties": false, "properties": properties})
}

pub(super) fn enterprise_annual_report() -> Value {
    let mut properties = entity_properties().as_object().cloned().unwrap_or_default();
    properties.insert("year".into(), json!({"type": "integer", "minimum": 1900, "maximum": 2100, "description": "年报自然年；请求时转成官方字符串"}));
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": ["year"]
    })
}

pub(super) fn listed_announcement_search() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "公告标题：按空格切分后需全部命中"},
            "name": {"type": "string", "description": "公司全称，子串模糊命中"},
            "jc": {"type": "string", "description": "股票简称，精确匹配"},
            "content": {"type": "string", "description": "公告全文检索词；空格拆分后按 search_mode 连接"},
            "search_mode": {"type": "string", "enum": ["AND", "OR"], "default": "AND", "description": "全文关键词拼接模式，仅作用于 content"},
            "fbrq_start": date("公告发布日期起，含当日，YYYY-MM-DD"),
            "fbrq_end": date("公告发布日期止，含当日，YYYY-MM-DD"),
            "market": {"type": "string", "enum": LISTED_MARKETS, "description": "交易所，精确匹配"},
            "area": {"type": "string", "enum": LISTED_AREAS, "description": "地区，精确匹配"},
            "zsx_type": {"type": "string", "description": "中上协行业分类，子串模糊命中"},
            "top_k": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20, "description": "返回条数；官方默认与上限均为 50"}
        }
    })
}

pub(super) fn hall_detect() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "text": {"type": "string", "description": "需要校验法规、法条和案号引用的原文"}
        },
        "required": ["text"]
    })
}
