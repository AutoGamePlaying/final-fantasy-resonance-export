-- 01_text indexes. Partial covering indexes accelerate short-query LIKE fallback per language
-- without touching the main `text` table or scanning NULL cells.

CREATE INDEX idx_text_ja ON text(ns, key, ja) WHERE ja IS NOT NULL;
CREATE INDEX idx_text_en ON text(ns, key, en) WHERE en IS NOT NULL;
CREATE INDEX idx_text_zh ON text(ns, key, zh) WHERE zh IS NOT NULL;
CREATE INDEX idx_text_untranslated_lang ON text_untranslated(lang, ns, key);
