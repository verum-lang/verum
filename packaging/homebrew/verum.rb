# Homebrew Formula for Verum Programming Language
class Verum < Formula
  desc "Practical language with refinement types and gradual verification"
  homepage "https://verum-lang.org"
  version "1.0.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/verum-lang/verum/releases/download/v1.0.0/verum-v1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_ARM64"
    else
      url "https://github.com/verum-lang/verum/releases/download/v1.0.0/verum-v1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_X86_64"
    end
  end

  on_linux do
    url "https://github.com/verum-lang/verum/releases/download/v1.0.0/verum-v1.0.0-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "REPLACE_WITH_ACTUAL_SHA256_LINUX"
  end

  depends_on "llvm@18"
  depends_on "z3"

  def install
    bin.install "verum"

    # Install shell completions if available
    if File.exist?("completions")
      bash_completion.install "completions/verum.bash" => "verum"
      zsh_completion.install "completions/_verum"
      fish_completion.install "completions/verum.fish"
    end

    # Install man pages if available
    if File.exist?("man")
      man1.install Dir["man/*.1"]
    end
  end

  test do
    # Test that verum runs and reports correct version
    assert_match "verum 1.0.0", shell_output("#{bin}/verum --version")

    # Test basic compilation
    (testpath/"hello.vr").write <<~EOS
      fn main() {
          print("Hello, Homebrew!")
      }
    EOS

    system bin/"verum", "check", "hello.vr"
  end
end
