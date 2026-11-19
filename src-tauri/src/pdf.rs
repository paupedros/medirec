use printpdf::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const MAX_CHARS: usize = 72;
const FONT_SIZE: f32 = 14.0;
const LINE_MM: f32 = 7.0;

fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for w in para.split_whitespace() {
            if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > MAX_CHARS {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(w);
        }
        lines.push(cur);
    }
    lines
}

pub fn export(path: &Path, title: &str, date: &str, text: &str) -> Result<(), String> {
    let (doc, p1, l1) = PdfDocument::new(title, Mm(210.0), Mm(297.0), "Capa");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica).map_err(|e| e.to_string())?;
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).map_err(|e| e.to_string())?;

    let mut layer = doc.get_page(p1).get_layer(l1);
    layer.use_text(title, 22.0, Mm(20.0), Mm(275.0), &bold);
    layer.use_text(date, 12.0, Mm(20.0), Mm(267.0), &font);
    let mut y = 255.0f32;

    for line in wrap(text) {
        if y < 20.0 {
            let (p, l) = doc.add_page(Mm(210.0), Mm(297.0), "Capa");
            layer = doc.get_page(p).get_layer(l);
            y = 277.0;
        }
        layer.use_text(line, FONT_SIZE, Mm(20.0), Mm(y), &font);
        y -= LINE_MM;
    }
    doc.save(&mut BufWriter::new(File::create(path).map_err(|e| e.to_string())?))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn wraps_long_text_and_keeps_paragraphs() {
        let lines = super::wrap(&"palabra ".repeat(40));
        assert!(lines.len() > 1 && lines.iter().all(|l| l.chars().count() <= super::MAX_CHARS));
        assert_eq!(super::wrap("a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn writes_spanish_text() {
        let p = std::env::temp_dir().join("medirec_test.pdf");
        super::export(&p, "Revisión", "hoy", "Él comió piña. ¿Qué día? ¡Ñu! áéíóú ÁÉÍÓÚ ü").unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > 500);
    }
}
