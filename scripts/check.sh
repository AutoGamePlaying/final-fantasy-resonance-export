#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PHYS_ROOT="$(pwd -P)"
# Resolve native Windows path in Git Bash / MSYS when available so .NET env defaults stay valid.
HOST_ROOT="$(pwd -W 2>/dev/null || pwd)"

CI_LOG_FILE=""
TEMP_AGP_HOME=""
cleanup_temp_state() {
  if [[ -n "$CI_LOG_FILE" && -f "$CI_LOG_FILE" ]]; then
    rm -f "$CI_LOG_FILE"
  fi
  if [[ -n "$TEMP_AGP_HOME" && -d "$TEMP_AGP_HOME" ]]; then
    rm -rf "$TEMP_AGP_HOME"
  fi
}
trap cleanup_temp_state EXIT

if [[ -z "${AGP_HOME:-}" ]]; then
  TEMP_AGP_HOME="$(mktemp -d "${TMPDIR:-/tmp}/ffr-check-agp.XXXXXX")"
  export AGP_HOME="$TEMP_AGP_HOME"
fi

emit_ci_annotations() {
  local step="$1"
  local log_file="$2"
  local exit_code="$3"

  FFR_CI_STEP="$step" \
  FFR_CI_EXIT_CODE="$exit_code" \
  FFR_CI_ROOT_1="$ROOT" \
  FFR_CI_ROOT_2="$PHYS_ROOT" \
  FFR_CI_ROOT_3="$HOST_ROOT" \
  FFR_CI_ROOT_4="${GITHUB_WORKSPACE:-}" \
  LC_ALL=C awk '
    function trim(s) {
      sub(/^[ \t]+/, "", s)
      sub(/[ \t]+$/, "", s)
      return s
    }

    function starts_with(s, prefix) {
      return substr(s, 1, length(prefix)) == prefix
    }

    function collapse_spaces(s) {
      gsub(/\t/, " ", s)
      gsub(/  +/, " ", s)
      return trim(s)
    }

    function replace_literal(s, pat, rep,    idx, out, plen) {
      if (pat == "") return s
      plen = length(pat)
      out = ""
      while ((idx = index(s, pat)) > 0) {
        out = out substr(s, 1, idx - 1) rep
        s = substr(s, idx + plen)
      }
      return out s
    }

    function replace_literal_ci(s, pat, rep,    lpat, idx, out, plen) {
      if (pat == "") return s
      lpat = tolower(pat)
      plen = length(pat)
      out = ""
      while ((idx = index(tolower(s), lpat)) > 0) {
        out = out substr(s, 1, idx - 1) rep
        s = substr(s, idx + plen)
      }
      return out s
    }

    function add_pat(p, ci,    i) {
      if (length(p) <= 1) return
      for (i = 1; i <= n_pats; i++) {
        if (pats[i] == p && pat_ci[i] == ci) return
      }
      n_pats++
      pats[n_pats] = p
      pat_ci[n_pats] = ci
    }

    function register_root(raw,    slash, bs, bbs, c3) {
      if (raw == "") return
      while (length(raw) > 1 && (substr(raw, length(raw), 1) == "/" || substr(raw, length(raw), 1) == "\\")) {
        raw = substr(raw, 1, length(raw) - 1)
      }
      if (length(raw) <= 1) return

      # Strip optional Windows verbatim path prefix before generating all slash/escaped forms.
      if (substr(raw, 1, 4) == "\\\\?\\" || substr(raw, 1, 4) == "//?/") {
        raw = substr(raw, 5)
      }

      c3 = substr(raw, 3, 1)
      if (raw ~ /^[A-Za-z]:/ && (c3 == "/" || c3 == "\\")) {
        slash = replace_literal(raw, "\\", "/")
        bs = replace_literal(slash, "/", "\\")
        bbs = replace_literal(slash, "/", "\\\\")
        add_pat("\\\\\\\\?\\\\" bbs, 1)
        add_pat("\\\\?\\" bs, 1)
        add_pat(bbs, 1)
        add_pat(bs, 1)
        add_pat(slash, 1)
      } else {
        add_pat(raw, 0)
      }
    }

    function sort_pats(    i, j, tmp_p, tmp_ci) {
      for (i = 2; i <= n_pats; i++) {
        tmp_p = pats[i]
        tmp_ci = pat_ci[i]
        j = i - 1
        while (j >= 1 && length(pats[j]) < length(tmp_p)) {
          pats[j + 1] = pats[j]
          pat_ci[j + 1] = pat_ci[j]
          j--
        }
        pats[j + 1] = tmp_p
        pat_ci[j + 1] = tmp_ci
      }
    }

    function sanitize(s,    i) {
      gsub(/\033\[[0-9;?]*[A-Za-z]/, "", s)
      gsub(/\r/, "", s)
      s = collapse_spaces(s)
      for (i = 1; i <= n_pats; i++) {
        if (pat_ci[i]) {
          s = replace_literal_ci(s, pats[i], "<repo>")
        } else {
          s = replace_literal(s, pats[i], "<repo>")
        }
      }
      return s
    }

    function escape_prop(s) {
      s = replace_literal(s, "%", "%25")
      s = replace_literal(s, "\r", "%0D")
      s = replace_literal(s, "\n", "%0A")
      s = replace_literal(s, ":", "%3A")
      s = replace_literal(s, ",", "%2C")
      return s
    }

    function escape_data(s) {
      s = replace_literal(s, "%", "%25")
      s = replace_literal(s, "\r", "%0D")
      s = replace_literal(s, "\n", "%0A")
      return s
    }

    function truncate_utf8(s, max_len,    cut, b) {
      if (length(s) <= max_len) return s
      if (max_len <= 3) return substr(s, 1, max_len)
      cut = max_len - 3
      # Back off across UTF-8 continuation bytes so multibyte characters are never split.
      while (cut > 0) {
        b = substr(s, cut + 1, 1)
        if (b >= "\200" && b <= "\277") {
          cut--
        } else {
          break
        }
      }
      s = substr(s, 1, cut)
      # Avoid leaving a partial %25 escape at the truncation point.
      if (length(s) >= 1 && substr(s, length(s), 1) == "%") {
        s = substr(s, 1, length(s) - 1)
      } else if (length(s) >= 2 && substr(s, length(s) - 1, 2) == "%2") {
        s = substr(s, 1, length(s) - 2)
      }
      return s "..."
    }

    function emit(msg,    prefix, budget, body, line) {
      if (emitted >= max_annotations) return
      msg = sanitize(msg)
      if (msg == "") return
      prefix = "::error title=" title_esc "::"
      budget = max_line_len - length(prefix)
      if (budget <= 0) return
      body = truncate_utf8(escape_data(msg), budget)
      line = prefix body
      if (line in seen) return
      seen[line] = 1
      emitted++
      print line
    }

    function register_dotnet_test(name) {
      if (name == "") return
      if (!(name in dn_seen)) {
        dn_seen[name] = 1
        dn_count++
        dn_order[dn_count] = name
      }
    }

    function is_dotnet_detail_line(s) {
      return starts_with(s, "Expected:") || starts_with(s, "Actual:") || \
             starts_with(s, "String:") || starts_with(s, "RegEx:") || \
             starts_with(s, "Not found:") || starts_with(s, "In value:") || \
             starts_with(s, "Looking for:")
    }

    function record_dotnet_line(name, line) {
      if (name == "" || line == "") return
      if (dn_assert[name] == "" && index(line, "Assert.") > 0) {
        dn_assert[name] = line
        return
      }
      if (is_dotnet_detail_line(line)) {
        if (dn_extra_cnt[name] < 2) {
          dn_extra[name] = (dn_extra[name] == "" ? line : dn_extra[name] " | " line)
          dn_extra_cnt[name]++
        }
        return
      }
      if (dn_assert[name] == "" && dn_fallback[name] == "") {
        dn_fallback[name] = line
      }
    }

    function emit_dotnet_tests(    i, name, body) {
      for (i = 1; i <= dn_count; i++) {
        name = dn_order[i]
        if (dn_assert[name] != "") {
          body = name " | " dn_assert[name]
          if (dn_extra[name] != "") body = body " | " dn_extra[name]
        } else if (dn_fallback[name] != "") {
          body = name " | " dn_fallback[name]
          if (dn_extra[name] != "") body = body " | " dn_extra[name]
        } else {
          body = name " | test failed"
        }
        emit(body)
      }
    }

    function flush_rust(    body) {
      if (!in_rust) return
      if (rust_panic != "") {
        if (rust_details != "") {
          if (substr(rust_panic, length(rust_panic), 1) == ":") {
            body = rust_test " | " rust_panic " " rust_details
          } else {
            body = rust_test " | " rust_panic " | " rust_details
          }
        } else {
          body = rust_test " | " rust_panic
        }
      } else if (rust_fallback != "") {
        body = rust_test " | " rust_fallback
      } else {
        body = rust_test " | test failed"
      }
      emit(body)
      in_rust = 0
      rust_test = ""
      rust_panic = ""
      rust_details = ""
      rust_detail_count = 0
      rust_collecting = 0
      rust_fallback = ""
      rust_fallback_count = 0
    }

    function flush_compile() {
      if (!in_compile) return
      if (compile_loc != "") {
        emit(compile_err " | " compile_loc)
      } else {
        emit(compile_err)
      }
      in_compile = 0
      compile_err = ""
      compile_loc = ""
      compile_wait_lines = 0
    }

    BEGIN {
      max_annotations = 20
      max_line_len = 300
      emitted = 0
      n_pats = 0
      dn_count = 0
      dn_mode = ""
      dn_cur = ""
      dn_in_msg = 0
      title_esc = escape_prop(ENVIRON["FFR_CI_STEP"])
      exit_code = ENVIRON["FFR_CI_EXIT_CODE"]

      register_root(ENVIRON["FFR_CI_ROOT_1"])
      register_root(ENVIRON["FFR_CI_ROOT_2"])
      register_root(ENVIRON["FFR_CI_ROOT_3"])
      register_root(ENVIRON["FFR_CI_ROOT_4"])
      sort_pats()
    }

    {
      raw_line = $0
      gsub(/\033\[[0-9;?]*[A-Za-z]/, "", raw_line)
      gsub(/\r/, "", raw_line)
      tline = trim(raw_line)
      if (tline != "") last_nonempty = tline

      # Rust test failure block header.
      if (raw_line ~ /^---- .+ stdout ----$/) {
        flush_rust()
        dn_mode = ""
        dn_in_msg = 0
        flush_compile()
        in_rust = 1
        rust_test = substr(raw_line, 6, length(raw_line) - 17)
        rust_panic = ""
        rust_details = ""
        rust_detail_count = 0
        rust_collecting = 0
        rust_fallback = ""
        rust_fallback_count = 0
        next
      }

      # .NET xUnit test failure header (may or may not be followed inline by Assert. lines).
      dline = raw_line
      sub(/^[ \t]*\[xUnit\.net [^]]*\][ \t]*/, "", dline)
      dline = trim(dline)
      if (dline ~ /\[FAIL\]$/ || dline ~ /^\[FAIL\][ \t]+/) {
        flush_rust()
        flush_compile()
        dotnet_test = dline
        sub(/[ \t]*\[FAIL\]$/, "", dotnet_test)
        sub(/^\[FAIL\][ \t]+/, "", dotnet_test)
        dotnet_test = trim(dotnet_test)
        register_dotnet_test(dotnet_test)
        dn_mode = "xunit"
        dn_cur = dotnet_test
        dn_in_msg = 0
        next
      }

      # .NET VSTest detail header ("  Failed <TestName> [<duration>]") emitted with Error Message.
      if (raw_line ~ /^[ \t]+Failed .+ \[[^]]+\]$/) {
        flush_rust()
        flush_compile()
        vtest = raw_line
        sub(/^[ \t]+Failed /, "", vtest)
        sub(/ \[[^]]+\]$/, "", vtest)
        vtest = trim(vtest)
        register_dotnet_test(vtest)
        dn_mode = "vstest"
        dn_cur = vtest
        dn_in_msg = 0
        next
      }

      if (in_rust) {
        if (raw_line == "failures:" || starts_with(raw_line, "test result:") || raw_line ~ /^running [0-9]+ tests?$/) {
          flush_rust()
        } else {
          if (rust_panic == "" && index(tline, "panicked at ") > 0) {
            pidx = index(tline, "panicked at ")
            tprefix = "thread \047" rust_test "\047"
            if (substr(tline, 1, length(tprefix)) == tprefix && pidx > length(tprefix)) {
              rust_panic = substr(tline, pidx)
            } else {
              rust_panic = tline
            }
            rust_collecting = 1
          } else if (rust_collecting) {
            if (tline == "") {
              if (rust_detail_count > 0) rust_collecting = 0
            } else if (starts_with(tline, "note: run with ") || starts_with(tline, "stack backtrace:") || starts_with(tline, "note: Some details are omitted")) {
              rust_collecting = 0
            } else if (rust_detail_count < 5) {
              rust_details = (rust_details == "" ? tline : rust_details " | " tline)
              rust_detail_count++
            }
          } else if (rust_panic == "" && tline != "") {
            if (starts_with(tline, "Error:")) {
              rust_fallback = tline
              rust_fallback_count = 1
            } else if (rust_fallback_count < 2) {
              rust_fallback = (rust_fallback == "" ? tline : rust_fallback " | " tline)
              rust_fallback_count++
            }
          }
          next
        }
      }

      if (dn_mode == "xunit") {
        if (raw_line ~ /^[ \t]*(Passed!|Failed!|Total tests:|Test Run Failed|Starting test execution)/ || starts_with(dline, "Stack Trace:") || starts_with(dline, "Output:")) {
          dn_mode = ""
        } else if (raw_line ~ /^[ \t]*\[xUnit\.net /) {
          record_dotnet_line(dn_cur, dline)
          next
        } else {
          dn_mode = ""
        }
      }

      if (dn_mode == "vstest") {
        if (raw_line ~ /^[ \t]*(Passed!|Failed!|Total tests:|Test Run Failed|Starting test execution)/ || starts_with(tline, "Stack Trace:") || starts_with(tline, "Standard Output Messages:")) {
          dn_mode = ""
          dn_in_msg = 0
        } else if (tline == "Error Message:") {
          dn_in_msg = 1
          next
        } else if (dn_in_msg) {
          record_dotnet_line(dn_cur, tline)
          next
        }
      }

      if (in_compile) {
        if (tline ~ /^-->[ \t]+/) {
          compile_loc = tline
          flush_compile()
          next
        }
        compile_wait_lines++
        if (tline == "" || compile_wait_lines > 4 || raw_line ~ /^error(\[[^]]+\])?:/ || raw_line ~ /^warning(\[[^]]+\])?:/) {
          flush_compile()
        } else {
          next
        }
      }

      # Rust / Cargo compiler and clippy errors.
      if (raw_line ~ /^error(\[[^]]+\])?:[ \t]/) {
        if (raw_line ~ /^error:[ \t]+(test failed|[0-9]+ targets? failed|aborting due to |could not compile |build failed)/) {
          next
        }
        in_compile = 1
        compile_err = tline
        compile_loc = ""
        compile_wait_lines = 0
        next
      }

      # MSBuild / Roslyn C# compiler errors.
      if (tline ~ /: error ([A-Za-z0-9]+)?[ \t]*:/) {
        cs_err = tline
        sub(/[ \t]+\[[^][]+\.(csproj|sln)\]$/, "", cs_err)
        emit(cs_err)
        next
      }
    }

    END {
      flush_rust()
      flush_compile()
      emit_dotnet_tests()
      if (emitted == 0) {
        if (last_nonempty != "") {
          emit("exit code " exit_code " | " last_nonempty)
        } else {
          emit("exit code " exit_code)
        }
      }
    }
  ' "$log_file"
}

run_ci_logged() {
  local step="$1"
  shift

  if [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
    "$@"
    return $?
  fi

  CI_LOG_FILE="$(mktemp "${TMPDIR:-/tmp}/ffr-check-log.XXXXXX")"
  local rc=0
  set +e
  "$@" 2>&1 | tee "$CI_LOG_FILE"
  rc="${PIPESTATUS[0]}"
  set -e

  if [[ "$rc" -ne 0 ]]; then
    emit_ci_annotations "$step" "$CI_LOG_FILE" "$rc" || true
    rm -f "$CI_LOG_FILE"
    CI_LOG_FILE=""
    exit "$rc"
  fi

  rm -f "$CI_LOG_FILE"
  CI_LOG_FILE=""
}

if [[ -n "${DOTNET_ROOT:-}" ]]; then
  DOTNET_DIR="${DOTNET_ROOT%/}"
  DOTNET_DIR="${DOTNET_DIR%\\}"
  if [[ -x "$DOTNET_DIR/dotnet.exe" ]]; then
    DOTNET_BIN="$DOTNET_DIR/dotnet.exe"
  else
    DOTNET_BIN="$DOTNET_DIR/dotnet"
  fi
elif [[ -x "$ROOT/.deps/dotnet/dotnet" ]]; then
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet"
elif [[ -x "$ROOT/.deps/dotnet/dotnet.exe" ]]; then
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet.exe"
elif command -v dotnet >/dev/null 2>&1; then
  DOTNET_BIN="$(command -v dotnet)"
else
  export DOTNET_ROOT="$HOST_ROOT/.deps/dotnet"
  DOTNET_BIN="$ROOT/.deps/dotnet/dotnet"
fi

if [[ ! -x "$DOTNET_BIN" ]]; then
  echo "Error: dotnet executable not found: $DOTNET_BIN (install .NET 10 SDK into .deps/dotnet or set DOTNET_ROOT)" >&2
  exit 2
fi

export DOTNET_CLI_HOME="${DOTNET_CLI_HOME:-$HOST_ROOT/.deps/dotnet-home}"
export NUGET_PACKAGES="${NUGET_PACKAGES:-$HOST_ROOT/.deps/nuget}"
export NUGET_HTTP_CACHE_PATH="${NUGET_HTTP_CACHE_PATH:-$HOST_ROOT/.deps/nuget-http}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
# Do not leave MSBuild / compiler server processes running after the script exits: they would
# inherit file descriptors such as the shared `flock /tmp/ffpr-heavy.lock` and hold it forever.
export MSBUILDDISABLENODEREUSE=1
export DOTNET_CLI_USE_MSBUILD_SERVER=0
export UseSharedCompilation=false

# Format checks run on Linux only to avoid Windows line-ending noise in CI.
SKIP_FMT="${FFR_CHECK_SKIP_FMT:-0}"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    SKIP_FMT=1
    ;;
esac

if [[ "$SKIP_FMT" == "0" ]]; then
  echo "==> dotnet format --verify-no-changes"
  "$DOTNET_BIN" format FfrExport.sln --verify-no-changes
else
  echo "==> Skipping dotnet format (non-Linux environment or FFR_CHECK_SKIP_FMT is set)"
fi

echo "==> dotnet build -c Release"
run_ci_logged "dotnet build" "$DOTNET_BIN" build FfrExport.sln -c Release

echo "==> dotnet test"
run_ci_logged "dotnet test" "$DOTNET_BIN" test FfrExport.sln -c Release --no-build

# Prefer cargo already on PATH; fall back to CARGO_HOME or $HOME/.cargo/bin.
if ! command -v cargo >/dev/null 2>&1; then
  if [[ -n "${CARGO_HOME:-}" && -d "$CARGO_HOME/bin" ]]; then
    export PATH="$PATH:$CARGO_HOME/bin"
  elif [[ -d "$HOME/.cargo/bin" ]]; then
    export PATH="$PATH:$HOME/.cargo/bin"
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Error: cargo executable not found (ensure the Rust toolchain is on PATH or in \$HOME/.cargo/bin)" >&2
  exit 2
fi

if [[ "$SKIP_FMT" == "0" ]]; then
  echo "==> cargo fmt --check"
  cargo fmt --check
else
  echo "==> Skipping cargo fmt (non-Linux environment or FFR_CHECK_SKIP_FMT is set)"
fi

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
run_ci_logged "cargo clippy" cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
run_ci_logged "cargo test" cargo test --workspace
