//! Treaty of Babel iFiction metadata.
//!
//! One parser for two sources: a blorb's `IFmd` chunk and IFDB's
//! `viewgame?ifiction` response are the same format. IFDB additionally carries an
//! `<ifdb>` extension element in its own namespace, which an IFmd chunk lacks.

const BABEL_NS: &str = "http://babel.ifarchive.org/protocol/iFiction/";
const IFDB_NS: &str = "http://ifdb.org/api/xmlns";

/// Parsed iFiction. Every field is optional: an IFmd chunk may carry only a
/// subset, and thin metadata must never fail a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IFiction {
    pub ifids: Vec<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    /// From the ifdb.org extension namespace; absent in an IFmd chunk.
    pub ifdb: Option<IfdbExt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfdbExt {
    pub tuid: String,
    pub link: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug)]
pub enum IFictionError {
    Xml(roxmltree::Error),
    /// Well-formed XML that is not an iFiction document.
    NotIFiction,
}

/// Finds a direct child element named `name` in the Babel namespace (or no
/// namespace at all), and returns its trimmed text — `None` if absent or blank.
fn child_text<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == name
                && matches!(n.tag_name().namespace(), Some(BABEL_NS) | None)
        })
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Finds a direct child element named `name`, ignoring namespace. Used inside
/// the `<ifdb>` subtree, which is unambiguously scoped to `IFDB_NS` already.
fn child_text_any_ns<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn parse(xml: &[u8]) -> Result<IFiction, IFictionError> {
    let text = std::str::from_utf8(xml).map_err(|_| IFictionError::NotIFiction)?;
    let doc = roxmltree::Document::parse(text).map_err(IFictionError::Xml)?;

    let story = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "story")
        .ok_or(IFictionError::NotIFiction)?;

    let mut result = IFiction::default();

    if let Some(bibliographic) = story
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "bibliographic")
    {
        result.title = child_text(bibliographic, "title");
        result.author = child_text(bibliographic, "author");
        result.language = child_text(bibliographic, "language");
        result.first_published = child_text(bibliographic, "firstpublished");
        result.genre = child_text(bibliographic, "genre");
        result.description = child_text(bibliographic, "description");
    }

    if let Some(identification) = story
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "identification")
    {
        result.ifids = identification
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "ifid")
            .filter_map(|n| n.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }

    if let Some(ifdb) = story.children().find(|n| {
        n.is_element() && n.tag_name().name() == "ifdb" && n.tag_name().namespace() == Some(IFDB_NS)
    }) {
        if let Some(tuid) = child_text_any_ns(ifdb, "tuid") {
            let link = child_text_any_ns(ifdb, "link");
            let cover_url = ifdb
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "coverart")
                .and_then(|coverart| child_text_any_ns(coverart, "url"));
            result.ifdb = Some(IfdbExt { tuid, link, cover_url });
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZORK: &[u8] = include_bytes!("../tests/fixtures/ifdb-zork1.xml");

    /// The live IFDB response. Guards the many-<title> trap: the fixture has 26
    /// <title> elements, all but one inside <downloads><link>, and only
    /// <bibliographic><title> is the game's.
    #[test]
    fn parses_the_live_ifdb_response() {
        let f = parse(ZORK).expect("fixture parses");
        assert_eq!(f.title.as_deref(), Some("Zork I"), "the bibliographic title, not a download's");
        assert_eq!(f.author.as_deref(), Some("Marc Blank and Dave Lebling"));
        assert_eq!(f.first_published.as_deref(), Some("1980"));
        assert_eq!(f.genre.as_deref(), Some("Zorkian/Cave crawl"));
        assert!(
            f.description.as_deref().unwrap().starts_with("Many strange tales"),
            "the blurb: {:?}", f.description
        );
        assert!(f.ifids.contains(&"ZCODE-52-871125".to_string()), "IFDB groups editions: {:?}", f.ifids);
    }

    #[test]
    fn extracts_the_ifdb_extension_block() {
        let f = parse(ZORK).unwrap();
        let ext = f.ifdb.expect("ifdb extension present");
        assert_eq!(ext.tuid, "0dbnusxunq7fw5ro");
        assert_eq!(
            ext.cover_url.as_deref(),
            Some("https://ifdb.org/coverart?id=0dbnusxunq7fw5ro&version=45"),
            "entity-decoded: &amp; must become &"
        );
    }

    /// A thin IFmd chunk carrying only a title must parse, not error — most of
    /// the struct is legitimately absent.
    #[test]
    fn a_minimal_ifmd_chunk_parses_with_everything_else_none() {
        let xml = br#"<ifindex version="1.0" xmlns="http://babel.ifarchive.org/protocol/iFiction/">
            <story><bibliographic><title>Curses</title></bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Curses"));
        assert!(f.author.is_none() && f.description.is_none() && f.ifdb.is_none());
    }

    /// An IFmd chunk with no default namespace (some tools emit bare iFiction).
    /// Accepted: namespace-absent is not namespace-wrong.
    #[test]
    fn a_namespaceless_chunk_still_parses() {
        let xml = br#"<ifindex><story><bibliographic><title>Bare</title></bibliographic></story></ifindex>"#;
        assert_eq!(parse(xml).unwrap().title.as_deref(), Some("Bare"));
    }

    #[test]
    fn malformed_xml_errors_and_never_panics() {
        assert!(parse(b"<ifindex><story>").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"\xff\xfe\x00garbage").is_err());
    }

    /// Whitespace around element text is incidental in XML; a title of
    /// "\n  Zork I\n  " must not reach the UI.
    #[test]
    fn text_is_trimmed_and_blanks_become_none() {
        let xml = br#"<ifindex><story><bibliographic>
            <title>  Spaced  </title><author>   </author>
        </bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Spaced"));
        assert!(f.author.is_none(), "whitespace-only is absent, not Some(\"\")");
    }
}
