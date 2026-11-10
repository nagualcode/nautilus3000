# nautilus3000 {VERSION} — Jardim Sonoro (macOS, Linux e Windows)

O Nautilus Bundle da Audio Ease (2000–2013) reconstruído a partir dos binários originais: **RiverRun3K** (sintetizador granular), **DeepPhaseNine3K** (phaser de 24 notches) e **Periscope3K** (EQ de 32 bandas com zoom e espectroscópio). Os três motores reproduzem o código de máquina original **bit a bit** (57, 39 e 28 cenários idênticos), com uma interface nova em dois temas (Modern e Classic).

### 🖥 Formatos
- **macOS (Apple Silicon)**: AU v2, VST3, CLAP, apps standalone e as CLIs `riverrun-render`, `dpn-render`, `periscope-render`.
- **Linux (x86_64)**: CLAP, VST3, **LV2**, apps standalone e as CLIs.
- **Windows (x64)**: CLAP, VST3, apps standalone e as CLIs.

### 📦 Instalação

**macOS (Apple Silicon / arm64)**
- **`riverrun-{VERSION}-macos.pkg`**, **`deepphasenine-{VERSION}-macos.pkg`**, **`periscope-{VERSION}-macos.pkg`**: um instalador por plug-in.
- **`nautilus3000-{VERSION}-macos-arm64.zip`**: os três para cópia manual (AU, VST3, CLAP, Standalone, CLI).

Macs Intel compilam a partir do código-fonte. Os binários têm assinatura ad-hoc e não são notarizados; se o macOS bloquear, use `xattr -dr com.apple.quarantine <arquivo>`.

**Linux (x86_64)**
- **`riverrun-…`, `deepphasenine-…`, `periscope-{VERSION}-linux-x86_64.tar.gz`**: descompacte e rode `./install.sh`.
- **`nautilus3000-{VERSION}-linux-x86_64.zip`**: cópia manual para `~/.clap`, `~/.vst3`, `~/.lv2`.

Precisa de ALSA ou JACK e de X11/OpenGL em tempo de execução.

**Windows (x64)**
- **`riverrun-…`, `deepphasenine-…`, `periscope-{VERSION}-windows.exe`**: instaladores.
- **`nautilus3000-{VERSION}-windows-x64.zip`**: cópia manual para as pastas CLAP/VST3 do usuário.

Os instaladores não são assinados, então o SmartScreen pede "Mais informações → Executar assim mesmo" na primeira vez.

Guia de uso completo no [README](https://github.com/nagualcode/nautilus3000#readme).
