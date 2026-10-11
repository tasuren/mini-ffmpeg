//! FFmpeg を最小構成でビルドし、その内容をビルドマニフェストとして書き出すタスク。
//!
//! どの `.c` がコンパイルされるかは、FFmpeg の `Makefile` を読むだけでは決まらない。
//! 部品どうしの依存が `configure` の中にあり、有効にした部品が別の部品を裏で
//! 有効にするためである。そこで、実際に `configure` と `make` を実行し、その結果
//! から読み取る。
//!
//! 1. `configure` を、ツリーの外のビルドディレクトリで実行する。
//! 2. `make -n V=1` で、実行されるはずのコンパイルコマンドを得る。翻訳単位と、
//!    インクルードディレクトリやマクロ定義がここから分かる。
//! 3. `make` で実際にビルドする。コンパイラが翻訳単位ごとに書き出す依存関係
//!    ファイル（`.d`）から、include されたファイルが分かる。
//! 4. ビルドマニフェストを `cinrs_manifest.toml` に書き出し、`configure` と `make` の生成物の
//!    うち必要なものを `FFmpeg_generated/` にコピーする。
//!
//! 読み書きする場所は、すべて `mini-ffmpeg-sys` の中にある。FFmpeg のツリーは
//! サブモジュールの `FFmpeg/`。クレートのディレクトリの中に揃えているのは、
//! crates.io へ載せるときに、ツールでコピーせずに同梱できるようにするため。
//!
//! 結果はコミットする。そのため、利用者のビルドでは `configure` も `make` も
//! 要らない。

mod compile_command;
mod depfile;
mod manifest;

use std::{
    collections::BTreeSet,
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use self::{
    compile_command::{CompileCommand, CompileFlags},
    manifest::{IncludeDir, Manifest, to_slash_path},
};
use crate::{MANIFEST_FILE, Result, sys_crate_dir, workspace_root};

/// `configure` に渡す引数。取り込む FFmpeg の構成は、ここが唯一の定義になる。
const CONFIGURE_ARGS: &[&str] = &[
    // 部品、ライブラリ、プログラム、ドキュメントをすべて無効にする。libavutil
    // だけは有効のまま残るので、必要なライブラリと部品を下で足していく。
    "--disable-all",
    // マシンにある外部ライブラリを自動で有効にさせない。
    //
    // 省略しても `config.h` には `CONFIG_AUTODETECT 0` と出るが、これは未設定を
    // 0 と書き出しているだけで、自動検出は行われる。macOS では VideoToolbox や
    // zlib、Homebrew のライブラリまで有効になり、コンパイル対象が増え、
    // `config.h` が生成したマシンに依存してしまう。
    "--disable-autodetect",
    "--enable-avcodec",
    "--enable-avformat",
    // cinrs が取り込めるのは C だけなので、アセンブリによる最適化を切る。
    // `--disable-asm` だけでは、C のコードに埋め込まれたインラインアセンブリが
    // 有効のまま残る（`HAVE_INLINE_ASM` が 1 になる）ので、両方とも指定する。
    "--disable-asm",
    "--disable-inline-asm",
    "--disable-network",
    // スパイクの最初の 1 回だけスレッドを切り、問題が起きたときに原因を
    // 切り分けやすくしている。スレッドを切ると FFmpeg 内部の mutex が何もしない
    // 実装になるので、ライブラリとして使う構成では外す必要がある。
    "--disable-pthreads",
    // スパイクの対象。WAV から WAV への `-c copy` に必要な部品だけを有効にする。
    "--enable-demuxer=wav",
    "--enable-muxer=wav",
    "--enable-protocol=file,pipe",
];

/// FFmpeg のツリー（サブモジュール）。`mini-ffmpeg-sys` からの相対パス。
const FFMPEG_DIR: &str = "FFmpeg";

/// `configure` と `make` の生成物のコピー先。`mini-ffmpeg-sys` からの相対パス。
///
/// このタスクが丸ごと管理するディレクトリで、実行のたびに作り直す。手で
/// ファイルを置いても消える。
const GENERATED_DIR: &str = "FFmpeg_generated";

/// FFmpeg のビルドで、ツリーを指すシンボリックリンクの名前。
///
/// ツリーの外でビルドすると、`configure` がビルドディレクトリにこの名前の
/// リンクを作る。コンパイルコマンドのソースも、依存関係ファイルのパスも、
/// ツリー内のファイルはこのリンク越しのパス（`src/libavutil/log.c`）で書かれる。
const SOURCE_LINK: &str = "src";

/// このタスクが読み書きする場所。
struct Paths {
    /// FFmpeg のツリー。シンボリックリンクを解決した絶対パス。
    ffmpeg_src: PathBuf,

    /// `configure` と `make` を実行する、ツリーの外のディレクトリ。
    build_dir: PathBuf,

    /// `mini-ffmpeg-sys` クレートのディレクトリ。結果はこの中に書き出す。
    sys_crate: PathBuf,
}

impl Paths {
    fn locate() -> Result<Self> {
        let sys_crate = sys_crate_dir()?;
        let ffmpeg_src = sys_crate.join(FFMPEG_DIR);

        if !ffmpeg_src.join("configure").is_file() {
            return Err(format!(
                "{} に FFmpeg の configure がありません。\n\
                 サブモジュールを取得してください: git submodule update --init",
                ffmpeg_src.display(),
            )
            .into());
        }

        Ok(Self {
            // `configure` には絶対パスで渡す。コンパイルコマンドに現れる
            // インクルードディレクトリが同じ絶対パスで書かれるので、後で
            // ツリーからの相対パスに直すときに前方一致で判定できる。
            ffmpeg_src: ffmpeg_src.canonicalize()?,

            // `/target` は gitignore されているので、ビルドの中間生成物を置ける。
            build_dir: workspace_root()?.join("target/xtask/ffmpeg-build"),

            sys_crate,
        })
    }
}

pub fn run() -> Result<()> {
    let paths = Paths::locate()?;

    // 前回のビルドの `.o` や `.d` が残っていると、今の構成ではコンパイルされない
    // ファイルがビルドマニフェストに紛れ込む。毎回、空のディレクトリからやり直す。
    if paths.build_dir.exists() {
        fs::remove_dir_all(&paths.build_dir)?;
    }

    fs::create_dir_all(&paths.build_dir)?;

    println!("configure を実行しています...");
    configure(&paths)?;

    // 実際にビルドした後では、`make -n` は「やることが無い」としてコマンドを
    // 表示しない。必ずビルドの前に取得する。
    let commands = dry_run_compile_commands(&paths)?;

    println!("make を実行しています（翻訳単位 {} 個）...", commands.len());
    build(&paths)?;

    let manifest = collect(&paths, &commands)?;
    write_outputs(&paths, &manifest)?;

    println!(
        "{} に書き出しました。\n  \
         FFmpeg {}\n  \
         翻訳単位 {} 個、include されるだけのソース {} 個、ヘッダ {} 個、生成物 {} 個",
        paths.sys_crate.display(),
        manifest.ffmpeg_version,
        manifest.translation_units.len(),
        manifest.included_sources.len(),
        manifest.headers.len(),
        manifest.generated.len(),
    );

    Ok(())
}

fn configure(paths: &Paths) -> Result<()> {
    let mut command = Command::new(paths.ffmpeg_src.join("configure"));
    command.args(CONFIGURE_ARGS).current_dir(&paths.build_dir);

    run_logged(command, &paths.build_dir.join("xtask-configure.log")).map_err(|error| {
        // `configure` が失敗した理由の詳細は、`configure` 自身のログにある。
        format!(
            "{error}\n詳細: {}",
            paths.build_dir.join("ffbuild/config.log").display(),
        )
        .into()
    })
}

/// `make -n V=1` を実行し、表示されたコマンドから C のコンパイルコマンドを集める。
///
/// `-n` はコマンドを表示するだけで実行しない。`V=1` は FFmpeg の Makefile に
/// 対する指定で、付けないと `CC libavutil/log.o` のような省略表示になる。
fn dry_run_compile_commands(paths: &Paths) -> Result<Vec<CompileCommand>> {
    let output = Command::new("make")
        .args(["-n", "V=1"])
        .current_dir(&paths.build_dir)
        .output()
        .map_err(|error| format!("make を実行できません: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "make -n V=1 が失敗しました（{}）\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        )
        .into());
    }

    let stdout = String::from_utf8(output.stdout)?;
    let mut commands = Vec::new();

    for line in stdout.lines() {
        if let Some(command) = compile_command::parse_line(line)? {
            commands.push(command);
        }
    }

    if commands.is_empty() {
        return Err("make -n V=1 の出力にコンパイルコマンドがありません".into());
    }

    Ok(commands)
}

fn build(paths: &Paths) -> Result<()> {
    let jobs = thread::available_parallelism().map_or(1, usize::from);

    let mut command = Command::new("make");
    command
        .arg(format!("-j{jobs}"))
        .current_dir(&paths.build_dir);

    run_logged(command, &paths.build_dir.join("xtask-make.log"))
}

/// コマンドを実行し、標準出力と標準エラー出力をログファイルに書き出す。
///
/// `configure` も `make` も出力が多いので、端末には流さない。失敗したときは、
/// ログファイルの場所をエラーに含める。
fn run_logged(mut command: Command, log_path: &Path) -> Result<()> {
    let log = File::create(log_path)?;

    let status = command
        .stdout(log.try_clone()?)
        .stderr(log)
        .status()
        .map_err(|error| {
            format!(
                "{} を実行できません: {error}",
                command.get_program().to_string_lossy(),
            )
        })?;

    if !status.success() {
        return Err(format!(
            "{} が失敗しました（{status}）\nログ: {}",
            command.get_program().to_string_lossy(),
            log_path.display(),
        )
        .into());
    }

    Ok(())
}

/// ビルド結果とコンパイルコマンドから、ビルドマニフェストを組み立てる。
fn collect(paths: &Paths, commands: &[CompileCommand]) -> Result<Manifest> {
    check_built_objects(paths, commands)?;

    let flags = common_flags(commands)?;

    let mut translation_units = BTreeSet::new();
    let mut tree_dependencies = BTreeSet::new();
    let mut generated = BTreeSet::new();

    for command in commands {
        let Dependency::Tree(source) = Dependency::classify(&command.source) else {
            // ビルド中に生成した `.c` をコンパイルする構成がある。バイナリを C の
            // 配列に変換して埋め込むもので、`ffbuild/common.mak` の `RUN_BIN2C` を
            // 使う規則がこれに当たる。CUDA のフィルタ（`*.ptx.c`）、Vulkan の
            // シェーダ（`*.spv.c`）、Metal（`*.metallib.c`）、fftools のリソース
            // （`*.html.c` と `*.css.c`）がある。いずれも今の構成では有効に
            // ならないので、必要になった時点で扱いを決める。
            //
            // ハードコードされたテーブル（`--enable-hardcoded-tables`）は、ここには
            // 来ない。書き出されるのは `libavcodec/pcm_tables.h` のようなヘッダで、
            // 翻訳単位が include する生成物として、下の依存関係の読み取りで拾われる。
            return Err(format!(
                "FFmpeg のツリーの外にあるソースのコンパイルには未対応です: {}",
                command.source,
            )
            .into());
        };

        translation_units.insert(source);

        let depfile_path = paths.build_dir.join(&command.object).with_extension("d");
        let depfile = fs::read_to_string(&depfile_path)
            .map_err(|error| format!("{} を読めません: {error}", depfile_path.display()))?;

        for dependency in depfile::parse_dependencies(&depfile)? {
            match Dependency::classify(&dependency) {
                Dependency::Tree(path) => tree_dependencies.insert(path),
                Dependency::Generated(path) => generated.insert(path),
                Dependency::External => false,
            };
        }
    }

    // ツリー内の依存のうち、翻訳単位そのものを除いた残りを、ヘッダとそれ以外に
    // 分ける。それ以外に入るのは、他の `.c` から include されるテンプレートなど。
    let (headers, included_sources): (Vec<_>, Vec<_>) = tree_dependencies
        .into_iter()
        .filter(|path| !translation_units.contains(path))
        .partition(|path| path.ends_with(".h"));

    let include_dirs = flags
        .include_dirs
        .iter()
        .map(|raw| IncludeDir::classify(raw, &paths.ffmpeg_src))
        .collect::<Result<Vec<_>>>()?;

    Ok(Manifest {
        ffmpeg_version: read_ffmpeg_version(paths)?,
        configure_args: CONFIGURE_ARGS.iter().map(|arg| (*arg).to_owned()).collect(),
        host_arch: env::consts::ARCH.to_owned(),
        host_os: env::consts::OS.to_owned(),
        c_standard: flags.std.ok_or("コンパイルコマンドに -std= がありません")?,
        defines: flags.defines,
        include_dirs,
        translation_units: translation_units.into_iter().collect(),
        included_sources,
        headers,
        generated: generated.into_iter().collect(),
    })
}

/// 依存関係ファイルやコンパイルコマンドに書かれたパスが、どこのファイルかを表す。
enum Dependency {
    /// FFmpeg のツリーにあるファイル。ツリーからの相対パス。
    Tree(String),

    /// `configure` や `make` が生成したファイル。ビルドディレクトリからの相対パス。
    Generated(String),

    /// どちらでもないファイル。システムのヘッダや SDK の設定ファイルなど。
    External,
}

impl Dependency {
    fn classify(raw: &str) -> Self {
        let path = Path::new(raw);

        if path.is_absolute() {
            return Self::External;
        }

        // ビルドディレクトリで `make` を実行しているので、相対パスはすべて
        // ビルドディレクトリが起点。そのうち `src/` 以下がツリー内のファイル。
        match path.strip_prefix(SOURCE_LINK) {
            Ok(relative) => Self::Tree(to_slash_path(relative)),
            Err(_) => Self::Generated(to_slash_path(path)),
        }
    }
}

/// 全翻訳単位のフラグが同じであることを確かめ、その共通のフラグを返す。
///
/// ビルドマニフェストには、フラグを全体で 1 組だけ書き出す。翻訳単位ごとに違うフラグが
/// あるのにそれを捨てると、元のビルドと違う内容で取り込むことになる。
/// 違いが出た時点で、翻訳単位ごとに持つ形へ変える必要がある。
///
/// ビルド中に実行するホスト用のプログラムも、ここで検出される。たとえば
/// `--enable-hardcoded-tables` では、テーブルを書き出すプログラム
/// （`libavcodec/pcm_tablegen.c` など）が、ライブラリの翻訳単位と同じ形の
/// コマンドでコンパイルされる。コマンドの形では区別できないが、フラグが違う
/// （`-DHAVE_AV_CONFIG_H` が付かない）ので、翻訳単位としてビルドマニフェストに入る前に
/// ここでエラーになる。
fn common_flags(commands: &[CompileCommand]) -> Result<CompileFlags> {
    let first = &commands[0];

    if let Some(different) = commands.iter().find(|command| command.flags != first.flags) {
        return Err(format!(
            "翻訳単位によってコンパイル時のフラグが異なります。\n\
             {}: {:?}\n\
             {}: {:?}",
            first.source, first.flags, different.source, different.flags,
        )
        .into());
    }

    Ok(first.flags.clone())
}

/// `make -n` から読み取った翻訳単位と、実際にできたオブジェクトファイルが
/// 一致することを確かめる。
///
/// コンパイルコマンドの読み取りは、コマンドの形についての前提に依存している。
/// 前提が外れて翻訳単位を取りこぼした場合に、欠けたビルドマニフェストを黙って書き出さない
/// ための検査。
fn check_built_objects(paths: &Paths, commands: &[CompileCommand]) -> Result<()> {
    let expected: BTreeSet<String> = commands
        .iter()
        .map(|command| to_slash_path(Path::new(&command.object)))
        .collect();

    let mut built = BTreeSet::new();
    find_objects(&paths.build_dir, &paths.build_dir, &mut built)?;

    if expected == built {
        return Ok(());
    }

    let missing: Vec<_> = expected.difference(&built).collect();
    let unexpected: Vec<_> = built.difference(&expected).collect();

    Err(format!(
        "コンパイルコマンドとビルド結果が一致しません。\n\
         コマンドはあるがオブジェクトが無い: {missing:?}\n\
         オブジェクトはあるがコマンドが無い: {unexpected:?}",
    )
    .into())
}

/// `dir` 以下の `.o` を、`root` からの相対パスで集める。
fn find_objects(root: &Path, dir: &Path, objects: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        // `DirEntry::file_type` による `is_dir` はシンボリックリンクを辿らない。
        // ビルドディレクトリには FFmpeg のツリーを指す `src` というリンクがあり、
        // 辿るとツリー全体を走査してしまうので、この性質に頼っている。
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            find_objects(root, &path, objects)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "o") {
            let relative = path.strip_prefix(root).expect("root 以下を走査している");

            objects.insert(to_slash_path(relative));
        }
    }

    Ok(())
}

/// `make` が生成する `libavutil/ffversion.h` から、FFmpeg のバージョンを読む。
///
/// このヘッダのバージョンは、FFmpeg のビルドが `git describe` などから決めた
/// もので、取り込むコードが実際に名乗るバージョンと一致する。
fn read_ffmpeg_version(paths: &Paths) -> Result<String> {
    let path = paths.build_dir.join("libavutil/ffversion.h");
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("{} を読めません: {error}", path.display()))?;

    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("#define FFMPEG_VERSION ")
                .map(|value| value.trim().trim_matches('"').to_owned())
        })
        .ok_or_else(|| format!("{} に FFMPEG_VERSION がありません", path.display()).into())
}

fn write_outputs(paths: &Paths, manifest: &Manifest) -> Result<()> {
    let generated_dir = paths.sys_crate.join(GENERATED_DIR);

    // 構成を変えると、不要になる生成物が出る。古いファイルを残さないよう、
    // このタスクが管理するディレクトリごと作り直す。
    if generated_dir.exists() {
        fs::remove_dir_all(&generated_dir)?;
    }

    for generated in &manifest.generated {
        let destination = generated_dir.join(generated);

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        // 生成物には手を加えず、そのままコピーする。ビルドディレクトリと同じ
        // 相対パスに置くので、このディレクトリをインクルードディレクトリにすれば、
        // 元のビルドの `-I.` と同じ見え方になる。
        fs::copy(paths.build_dir.join(generated), &destination)?;
    }

    fs::write(paths.sys_crate.join(MANIFEST_FILE), manifest.to_toml())?;

    Ok(())
}
