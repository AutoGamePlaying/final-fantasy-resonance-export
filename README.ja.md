[English](README.md) | [简体中文](README.zh-CN.md) | **日本語**

# final-fantasy-resonance-export

**FINAL FANTASY RESONANCE** 向けのアセットエクスポート・ローカルデータベース構築ツール（C# / .NET 10 / CUE4Parse + Rust / SQLite）です。ゲームインストールフォルダ内の UE5 IoStore / Pak パッケージを読み取り専用でマウントし、データテーブル、多言語テキスト、アセット総インデックス、全非画像アセット、レベルおよびマップ接続グラフを JSON として出力し、決定論的なローカル SQLite データベース（`ffr.sqlite`）を構築します。

> 本リポジトリは **FINAL FANTASY RESONANCE** 専用であり、汎用化は行いません。ゲームファイル、`.usmap` マッピングファイル、およびエクスポート成果物はすべて `.deps/`（または `$FFR_DEPS`）に配置され、リポジトリには含めません。

## クイックスタート（Windows）

開発環境のインストールは不要です。パッケージ済みの Windows 版をダウンロードし、GUI から数クリックでエクスポートできます。

1. **アーカイブをダウンロード**：
   - **正式リリース**：リポジトリの **Releases** ページから最新の `ffr-export-<version>-win-x64.zip` をダウンロードします。
   - **最新の開発ビルド**：**Actions** ページで `main` ブランチの最新成功 **CI** 実行を開き、下部 **Artifacts** から `ffr-export-<version>-<short-sha>-win-x64`（30 日間保持）をダウンロードします。
2. **同じフォルダにすべて展開**：zip アーカイブ全体を 1 つのフォルダに展開し、`ffr-export-gui.exe`、`ffr-export.exe`、`ffr-db.exe`、`known-builds.json` が同じフォルダにある状態を保ちます（`ffr-export-gui.exe` だけを移動しないでください。アーカイブ内には英・中・日対応の `README-Windows.txt` も同梱されています）。
3. **`.usmap` マッピングファイルを準備**：`.usmap` はツールに同梱できないため、実行中のゲームからダンプする必要があります。ゲームフォルダやローカルキャッシュに `.usmap` がない場合は、GUI の **`.usmap` を自動取得（ゲームを一度起動）**（または `ffr-export.exe fetch-usmap`）を使うか、UE4SS で手動ダンプしてください。詳細は [`.usmap` の取得方法](#usmap-の取得方法) および [docs/usmap.md](docs/usmap.md) を参照してください。
4. **`ffr-export-gui.exe` をダブルクリックで起動**し、内容を確認して **エクスポート開始** をクリックします：
   - **表示言語の切り替え**：既定は英語です。ウィンドウ右上のプルダウン（**English** / **简体中文** / **日本語**）からいつでも即時切り替えでき、選択内容は `settings.json` に保存されます（子プロセスの CLI にも `--lang` が自動で渡されます）。
   - **ゲームの自動検出**：起動時に Steam ライブラリ全体をスキャンし、ゲーム名・Steam ビルド番号・インストールフォルダを表示します。見つからない場合は **ゲームフォルダを手動選択…** から指定できます。
   - **`.usmap` の自動検出／ワンクリック取得**：`%LOCALAPPDATA%\agp\ffr-export\usmap\*.usmap`、`FFRS\Binaries\Win64\ue4ss\*.usmap`、`FFRS\Binaries\Win64\*.usmap` の順に検索して最新のファイルを選択します。見つからない場合は **`.usmap` を自動取得（ゲームを一度起動）** ボタンが表示されます。
   - **統一データディレクトリ**：既定の出力先、アーカイブ済み `.usmap`、ダウンロードキャッシュ、ログ、GUI 設定はすべて `%LOCALAPPDATA%\agp\ffr-export\` に集約されます（[統一データディレクトリと `AGP_HOME`](#統一データディレクトリと-agp_home) を参照）。前回使った出力ルートと言語設定は次回起動時に自動復元されます（レジストリには書き込みません）。
   - **設定と UE4SS クリーンアップ**：**設定…** からデータディレクトリを確認でき、メインウィンドウの **UE4SS をクリーンアップ…** から本ツールが導入した UE4SS のアンインストール（または明示選択時の既存 UE4SS バックアップ移動）が行えます。
   - **進捗表示と検証**：実行中は進捗バーとログをリアルタイム表示し、**キャンセル** でバックグラウンドプロセスツリーを安全に停止できます。完了後はデータベースパス、ファイルハッシュ、内容ハッシュ、`known-builds.json` との `✓` / `✗` 照合結果、**出力フォルダを開く**・**ハッシュをコピー** ボタンを表示します。

> 既知ビルド一覧（`known-builds.json`）は「ゲームファイル + `.usmap`」の組み合わせのみを登録対象とします。リフレクションデータ（`.jmap`）はエクスポートツールの対象外です。研究用途で必要な場合は、`ffr-db build --jmap` を直接実行してください（[docs/database.md](docs/database.md) を参照）。

GUI の構成イメージ：

```text
+------------------------------------------------------------------------------+
| FINAL FANTASY RESONANCE データエクスポートツール     言語: [ 日本語      v ] |
+------------------------------------------------------------------------------+
| [1. ゲーム (FINAL FANTASY RESONANCE)]                                        |
|   ゲーム名                  | Steam ビルド | インストールフォルダ [手動選択…] |
| > FINAL FANTASY RESONANCE   | 24250053     | ...\common\...       [再スキャン]|
+------------------------------------------------------------------------------+
| [2. マッピングファイル (.usmap)]                                             |
|   .usmap（必須）: 自動検出 FFRS-5.6.1.usmap (2026-11-25)     [.usmap を選択…]|
+------------------------------------------------------------------------------+
| [3. 出力先]                                                                  |
|   出力フォルダ: [ %LOCALAPPDATA%\agp\ffr-export\output\ffr-24250053 ] [参照…]|
+------------------------------------------------------------------------------+
| [               エ ク ス ポ ー ト 開 始               ]       [ キャンセル ] |
| 状態: ステップ 3/5 — ffr-db build で SQLite データベース構築 [=========>   ] |
| +--------------------------------------------------------------------------+ |
| | ==> ステップ 1/5: ゲームバージョンと入力ハッシュを検出                   | |
| | ==> ステップ 2/5: ゲームデータを JSON にエクスポート                     | |
| | ==> ステップ 3/5: ffr-db build で SQLite データベースを構築              | |
| +--------------------------------------------------------------------------+ |
+------------------------------------------------------------------------------+
| [4. 完了結果]  ✓ 登録済みビルドと一致 (ゲームファイル + .usmap)              |
|   データベース:  [ ...\ffr-24250053\ffr.sqlite       ] [ 出力フォルダを開く ]|
|   ファイルハッシュ:[ b0a228269588...                 ] [  ハッシュをコピー  ]|
|   内容ハッシュ:  [ 28374329602c...                   ]                       |
+------------------------------------------------------------------------------+
```

### 言語選択

GUI（`ffr-export-gui`）と CLI（`ffr-export`）は共通のローカライズリソースを使用し、**英語（`en`、既定）**・**簡体字中国語（`zh-Hans`）**・**日本語（`ja`）** に対応しています。OS のロケールに応じた自動切り替えは行わず、以下の優先順位で言語を決定します。

1. コマンドライン引数：`--lang en|zh-Hans|ja`（`locres` サブコマンドと併用する場合は、`locres --lang` がゲーム内テキスト言語の選択に使われるため `locres` より前に指定してください）
2. 環境変数：`AGP_LANG`
3. GUI に保存された設定：`<AGP_HOME>/ffr-export/settings.json` の `"language"`
4. 既定値：`en`

GUI から CLI 子プロセスを起動する際は `--lang <現在の言語>` と `--progress-json` を渡し、言語非依存のプロトコル（`@@progress {"step":n,"total":m,...}`）でプログレスバーを制御します。

### 統一データディレクトリと `AGP_HOME`

GUI・`build-db`・対話型ウィザード・`fetch-usmap` の既定ファイルはすべて 1 つのディレクトリ配下に集約されます：

```text
%LOCALAPPDATA%\agp\ffr-export\
  output\ffr-<Steam ビルド番号>\  既定の出力先（ffr.sqlite、ffr.sqlite.build.json、--keep-json 時の json\）
  usmap\                          自動取得または手動コピーした .usmap（最優先で自動検出）
  downloads\                      公式 UE4SS アーカイブのダウンロードキャッシュ（再利用前に SHA-256 検証）
  logs\                           GUI・CLI の実行ログ（最新 10 件のみ保持）
  settings.json                   GUI 設定（出力ルート、言語選択）
```

- **ルートの優先順位**：環境変数 `AGP_HOME`（そのまま `agp` 層として使用）→ Windows では `%LOCALAPPDATA%\agp` → Linux/macOS では `$XDG_DATA_HOME/agp`（未設定時は `~/.local/share/agp`）。低整合性（Low IL）や `%LOCALAPPDATA%` 書き込み不可の Windows 環境では、`%USERPROFILE%\AppData\LocalLow\agp`、`<exeDir>\agp-data`、`%USERPROFILE%\agp`、保存済み `agp-location.txt` の順にフォールバックし、`%TEMP%` には一切書き込みません。
- **明示パスの優先**：CLI の `--out` や GUI の **参照…** で指定したフォルダが常に優先され、**出力フォルダを開く** は実際の出力先を開きます。
- **旧設定の移行**：旧版の `%APPDATA%\ffr-export\settings.json` が存在し新設定が存在しない場合、初回起動時に自動移行し、旧ファイルは削除せず `settings.json.migrated` にリネームします。

### `.usmap` の取得方法

詳細な仕組み、変更ファイル一覧、トラブルシューティングは [docs/usmap.md](docs/usmap.md) を参照してください。

#### 方法 1：ワンクリック自動取得（Windows 推奨）

- **GUI**：`ffr-export-gui.exe` を起動し、`.usmap` が見つからない場合に表示される **`.usmap` を自動取得（ゲームを一度起動）** をクリックします。確認ダイアログを承認して完了を待つと、取得した `.usmap` が自動選択されます（途中でキャンセルしても一時 Mod を削除してゲームを閉じます）。
- **CLI**：

```powershell
ffr-export.exe fetch-usmap --game "<Steam ゲームフォルダ>" [--timeout 300] [--yes]
```

承認後にのみ公式 `UE4SS v4.0.0` を固定 URL からダウンロードし（サイズと SHA-256 を検証。ユーザーが導入済みの UE4SS 設定には一切触れません）、`enabled.txt` を使った一時読み取り専用 Mod を配置（`mods.txt` は未変更）、Steam 経由でゲームを起動してタイトル画面で `.usmap` をダンプし、`%LOCALAPPDATA%\agp\ffr-export\usmap\` に保存・`known-builds.json` と照合したのち、一時 Mod を削除してセーブデータに触れずゲームを終了します。UE4SS 本体は再利用のため残りますが、いつでもアンインストール可能です：

```powershell
ffr-export.exe uninstall-ue4ss --game "<Steam ゲームフォルダ>"
```

または GUI の **UE4SS をクリーンアップ…** を使用できます（既定では本ツールが記録したファイルのみ削除し、ユーザー導入済みの UE4SS は保持します）。

#### 方法 2：手動取得

[RE-UE4SS v4.0.0](https://github.com/UE4SS-RE/RE-UE4SS/releases/tag/v4.0.0) をダウンロードして `<ゲームフォルダ>\FFRS\Binaries\Win64\` に展開し、ゲーム起動後に UE4SS で `.usmap` をダンプして `FFRS\Binaries\Win64\ue4ss\` に残すか `%LOCALAPPDATA%\agp\ffr-export\usmap\` へコピーしてください。

### コンソールウィザードと CLI（Windows 任意）

同じフォルダ内の `ffr-export.exe` を端末やスクリプトから直接使うこともできます：

- **対話型ウィザード**：引数なしで `ffr-export.exe` を実行すると（`--lang ja` または `AGP_LANG=ja` 指定可）、案内に沿ってゲーム・`.usmap`・出力先を選んでエクスポートと DB 構築を行えます。
- **ワンコマンド DB 構築**：`build-db` サブコマンドを直接実行できます（GUI が内部で呼び出すコマンドと同一です）：

```powershell
ffr-export.exe --lang ja build-db --game "<Steam ゲームフォルダ>" --usmap "<Mappings.usmap>" [--out "<出力フォルダ>"]
```

## CLI と開発者向け手順

### 環境構築

#### 1. .NET 10 SDK の導入

既定ではリポジトリ直下の `.deps/dotnet` を使用します（NuGet キャッシュは `.deps/nuget` に配置され、`~/.dotnet` は変更しません）：

```bash
mkdir -p .deps
curl -fsSL https://dot.net/v1/dotnet-install.sh -o .deps/dotnet-install.sh
bash .deps/dotnet-install.sh --channel 10.0 --install-dir .deps/dotnet --no-path
```

既存の .NET 10 SDK を `DOTNET_ROOT` 環境変数で指定することもでき、`scripts/run.sh` と `scripts/check.sh` が自動認識します。

#### 2. ゲームファイルと `.usmap` の準備

- **ゲームファイル（読み取り専用）**：ゲームインストール先を `.deps/ffr/files/` に配置（またはシンボリックリンク）します。既定のマウント先は `$FFR_DEPS/ffr/files/FFRS/Content/Paks`（未設定時は `.deps/ffr/files/FFRS/Content/Paks`）で、ゲームファイルは一切変更しません。
- **`.usmap` マッピングファイル**：`list`・`raw`・`locres`・`inventory` は `.usmap` 不要、`export`・`export-assets`・`export-maps` は `--usmap <ファイル>` が必要です。

## パス規約と `FFR_DEPS`

開発用サブコマンドの既定入出力パスはすべて環境変数 `FFR_DEPS`（下流リポジトリ `Final-Fantasy-Resonance-Agent` と共通）を優先し、未設定時はリポジトリ内の `.deps/` を使用します。

### 既定の出力レイアウト

`DEPS="${FFR_DEPS:-.deps}"` とすると：

| パス | サブコマンド | 説明 |
| --- | --- | --- |
| `$DEPS/ffr/files/FFRS/Content/Paks` | 全コマンド（既定入力） | ゲーム IoStore / Pak フォルダ（読み取り専用、`--paks` で上書き可） |
| `$DEPS/ffr/export` | `export` | DataTable / StringTable の JSON 出力と `manifest.json` |
| `$DEPS/ffr/export/Localization` | `locres` | 多言語テキスト `<lang>.json` と `manifest.json` |
| `$DEPS/ffr/inventory` | `inventory` | アセットインデックス `registry.jsonl`、`summary.json`、`manifest.json` |
| `$DEPS/ffr/export-full` | `export-assets` | 全非画像アセット JSON、無加工コピーの `Config/*.ini`/`.uplugin`/`.uproject`、`manifest.json` |
| `$DEPS/ffr/export-full/maps` | `export-maps` | レベル `<Level>.json`、マップ接続グラフ `_graph.json`、`manifest.json` |
| `$DEPS/ffr/db/ffr.sqlite` | `ffr-db build` | 出力 JSON と任意の `.jmap`/スナップショットを統合したローカル SQLite データベース |

### 出力ディレクトリ保護ガード

- `--out` は `$FFR_DEPS`（または `.deps/`）配下に限定され、保護ディレクトリ（`ffr`、`ffr/files`、`r1`、`dotnet`、`nuget`、`dotnet-home`、`nuget-http` とその配下）は拒否されます。
- 非レジューム型コマンド（`export`、`locres`、`inventory`）は空でないフォルダを拒否し、`--force` 時も同一サブコマンドの `manifest.json`（`cue4ParseVersion` 付き）を確認してから削除します。
- レジューム型コマンド（`export-assets`、`export-maps`）は `--force` なしで既存ファイルをスキップし、`export-assets --force` でも他サブコマンドの `manifest.json` を持つサブフォルダ（`maps/` など）を保護します。

## サブコマンドとオプション

各サブコマンドは `scripts/run.sh [--lang en|zh-Hans|ja] <subcommand> [options...]` で実行します：

### 1. `list` — パッケージ内ファイル一覧

```bash
scripts/run.sh list [--paks <dir>] [--prefix <prefix>]...
```

一致した仮想パスとバイト数および総数を出力します。既定プレフィックスは `FFRS/Content/Datatable/` と `FFRS/Content/L10N/` です。

### 2. `raw` — 単一パッケージの生バイト抽出

```bash
scripts/run.sh raw <package-path> --out <file> [--paks <dir>]
```

指定した仮想パスの生バイト列を `--out`（`$FFR_DEPS` / `.deps/` 配下）へ書き出し、長さと SHA-256 を表示します。

### 3. `export` — データテーブルの JSON エクスポート

```bash
scripts/run.sh export --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--paks <dir>] [--force]
```

既定プレフィックスは `FFRS/Content/Datatable/`・`FFRS/Content/L10N/`、既定出力先は `$DEPS/ffr/export` です。2 スペースインデント・UTF-8（BOM なし）・LF 改行で出力し、個別ファイルの失敗は `manifest.json` の `failures` に記録して残りを継続します（失敗が 1 件以上ある場合、終了コードは `1`）。

### 4. `locres` — 多言語テキストのエクスポート

```bash
scripts/run.sh locres [--lang <game-langs>]... [--out <dir>] [--paks <dir>] [--force]
```

CUE4Parse の `FTextLocalizationResource` で `FFRS/Content/Localization/Game/<lang>/Game.locres` を読み込み、既定で `zh-Hans`・`en`・`ja` を `$DEPS/ffr/export/Localization/<lang>.json`（`{ namespace: { key: text } }` 形式）と `manifest.json` に出力します。`--lang` は複数指定またはカンマ区切り（例：`--lang zh-Hans,en,ja`）に対応します。

### 5. `inventory` — 全アセットインデックス生成

```bash
scripts/run.sh inventory [--paks <dir>] [--out <dir>] [--force]
```

`FFRS/AssetRegistry.bin`（`FAssetRegistryState`）と重複排除済み VFS マウントテーブルを結合し、`registry.jsonl`・`summary.json`・`manifest.json` を `$DEPS/ffr/inventory` に出力します。詳細は [docs/data-inventory.md](docs/data-inventory.md) を参照してください。

### 6. `export-assets` — 全非画像アセットのエクスポート

```bash
scripts/run.sh export-assets --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--jobs <n>] [--paks <dir>] [--force]
```

[docs/data-inventory.md](docs/data-inventory.md) の分類規則に従い、`FFRS/` 配下の非画像・非レベル・非大容量バイナリアセットを並列エクスポートし、`Config/*.ini`・`.uplugin`・`.uproject` を無加工コピーします。途中再開、`ContainerHeader` 欠落パッチへの自動フォールバック、単一ファイルエラー隔離に対応します。

分類確認モード（オフライン・JSON 直接出力）：

```bash
scripts/run.sh export-assets --classify DataTable,Texture2D,BlueprintGeneratedClass
```

### 7. `export-maps` — 全レベル出力とマップ接続グラフ

```bash
scripts/run.sh export-maps --usmap <Mappings.usmap> \
  [--out <dir>] [--prefix <prefix>]... [--map-data <CDT_MapData_Prod.json>] \
  [--jobs <n>] [--paks <dir>] [--force]
```

`FFRS/Content/` 配下の全 `.umap` レベルを走査してゲーム Actor とエンジン Actor を分離し、NPC・宝箱・ショップ・宿屋・セーブポイント・遷移トリガー・遷移ポイント・イベントトリガー・ギミック・エンカウント領域の要約と `_graph.json` 接続グラフを生成します（詳細は [docs/maps.md](docs/maps.md)）。テスト用の `--from-json <fixture-dir>` オフラインモードも備えています。

## JSON → SQLite（`ffr-db`）

リポジトリ内の Rust データ層（`crates/ffr-data`）と DB 構築 CLI（`crates/ffr-db`）により、エクスポート済み JSON と任意の `.jmap` リフレクションダンプ・実機スナップショットを単一の SQLite ファイルへ統合できます。スキーマと API 設計は [docs/database.md](docs/database.md) を参照してください。

### 使い方

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  [--export <dir>] [--full <dir>] [--inventory <dir>] [--maps <dir>] \
  [--jmap <file>]... [--snapshots <dir>] [--stages <list>] [--out <file>]
```

研究用途ビルド（任意の `.jmap` を追加。エクスポートツール外での利用、詳細は [docs/database.md](docs/database.md) を参照）：

```bash
FFR_DEPS=.deps cargo run --release -p ffr-db -- build \
  --jmap .deps/r1/<title-jmap>/<Mappings>.jmap \
  --jmap .deps/r1/<battle-jmap>/<Mappings>.jmap
```

- **既定入力**：`$FFR_DEPS/ffr/` 配下（`export`、`export-full`、`inventory`、`export-full/maps`）から読み込み、各フラグで個別上書き可能。
- **明示入力**：`--jmap`（複数回指定可、重複時は後勝ち）と `--snapshots` は明示指定時のみ実行。
- **既定出力**：`$FFR_DEPS/ffr/db/ffr.sqlite`（`--out` で変更可）。一時ファイルでインデックス作成・`ANALYZE`・`VACUUM` まで完了させてから原子的に置換します。

読み取り専用の確認サブコマンドとして `stats`、`sql "<SELECT ...>"`、`explain "<SELECT ...>"`、`hash`、`bench` も利用できます。

## `Final-Fantasy-Resonance-Agent` との連携

下流リポジトリ `../Final-Fantasy-Resonance-Agent` の `.deps` を `FFR_DEPS` に指定するだけで、一式を直接出力できます：

```bash
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap \
  --prefix FFRS/Content/Datatable/ --force

FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh locres --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh inventory --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-assets \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap --force
FFR_DEPS=../Final-Fantasy-Resonance-Agent/.deps scripts/run.sh export-maps \
  --usmap ../Final-Fantasy-Resonance-Agent/.deps/r1/<dump-dir>/<Mappings>.usmap --force
```

## ビルドとテスト

```bash
scripts/check.sh
```

以下を順に実行します：

1. `dotnet format FfrExport.sln --verify-no-changes`（Linux）
2. `dotnet build FfrExport.sln -c Release`（`Nullable` 有効・警告をエラー扱い）
3. `dotnet test FfrExport.sln -c Release --no-build`（xUnit、合成テストデータのみ使用）
4. `cargo fmt --check`（Linux）
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `cargo test --workspace`（`ffr-data`・`ffr-db` テスト。実データがない環境では実データテストを自動スキップ）

## CI とリリース

GitHub Actions 上で `ubuntu-latest` と `windows-latest` のネイティブツールチェーンを用いた継続的インテグレーションと自動リリースを行います：

- **継続的インテグレーション（`.github/workflows/ci.yml`）**：`check`（Windows での書き込み不可 `TEMP` 下 GUI セルフテスト含む）、`fixture-hash`（合成 DB と合成エクスポートマニフェスト）、`compare-hash`（Linux・Windows 間のバイト一致検証）、`package-windows`（`main` プッシュ時の Windows x64 ネイティブパッケージングと単一ファイル多言語セルフテスト、30 日間保存）。
- **リリース手順（`.github/workflows/release.yml`）**：リポジトリ直下の `VERSION` を更新して一致する `v*` タグをプッシュすると、`windows-latest` 上でチェック・パッケージング・単一ファイルセルフテストを実行し、`ffr-export-<version>-win-x64.zip`、`SHA256SUMS`、`known-builds.json` を添付した GitHub Release を自動作成します（ローカル Linux クロスビルド用 `scripts/package-windows.sh` も引き続き利用可能です）。

## Oodle とネットワーク通信ポリシー

- **Oodle 展開**：CUE4Parse に同梱されたオープンソース純 C# 実装 `OodleSharp` のみを使用します。`OodleHelper.Initialize` や `DownloadOodleDll` は一切呼び出しません。
- **ネットワーク通信の範囲**：
  - **エクスポートと DB 構築は完全オフライン**：`list`、`raw`、`export`、`locres`、`inventory`、`export-assets`、`export-maps`、`build-db`、`ffr-db` はネットワーク通信を行いません。
  - **ワンクリック `.usmap` 取得時のみ通信**：ユーザーが明示的に `fetch-usmap` または GUI の **`.usmap` を自動取得** を実行して承認し、既存 UE4SS および検証済みキャッシュが存在しない場合に限り、固定 URL `https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip` から HTTPS ダウンロードし、正確なバイト数と SHA-256 を検証します。

## ライセンス

[MIT License](LICENSE)
