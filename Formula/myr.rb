class Myr < Formula
  desc "Terminal-first MySQL/MariaDB schema and data explorer"
  homepage "https://github.com/juanicastellan0/myr"
  url "https://github.com/juanicastellan0/myr.git",
      tag: "v0.1.1",
      revision: "1d227f07429351c4b04ff9f3e6e35ce2ee4e1864"
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
