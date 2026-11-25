-- 00_meta indexes (created after bulk data import).

CREATE INDEX idx_build_issue_stage_kind ON build_issue(stage, kind);
CREATE INDEX idx_build_issue_kind ON build_issue(kind);
CREATE INDEX idx_build_issue_subject ON build_issue(subject);
