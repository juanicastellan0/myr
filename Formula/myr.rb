class Myr < Formula
  desc "Terminal-first MySQL/MariaDB schema and data explorer"
  homepage "https://github.com/juanicastellan0/myr"
  url "https://github.com/juanicastellan0/myr.git",
      tag: "v0.2.0-alpha.1",
      revision: "18ca0dfc9b2be75fe2f91316450cd1d1705cd1d3"
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
