-- 01_text indexes. The short-query LIKE fallback scans this index instead of the text table.

CREATE INDEX idx_text_lang_ns_key ON text(lang, ns, key);
CREATE INDEX idx_text_untranslated_lang ON text_untranslated(lang, ns, key);
