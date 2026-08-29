class Myr < Formula
  desc "Terminal-first MySQL/MariaDB schema and data explorer"
  homepage "https://github.com/juanicastellan0/myr"
  url "https://github.com/juanicastellan0/myr.git",
      tag: "v0.2.0-alpha.1",
      revision: "d6b830a9d1ef0f89d72b94ac0e2651783927e5fd"
  license "MIT"
  head "https://github.com/juanicastellan0/myr.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", *std_cargo_args(path: "app")
  end

  test do
    assert_match "Usage:", shell_output("#{bin}/myr-app --help")
  end
end
