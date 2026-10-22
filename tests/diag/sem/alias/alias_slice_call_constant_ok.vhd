-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2, 8.1, 9.4.3: a constant is a globally static primary whatever its initial value, so
-- an alias slice bounded by one is a static name (sim1, sim2, sim3 and sim4 accept)
entity e is
  generic (g : natural := 8);
end entity;
architecture a of e is
  impure function iw(x : natural) return natural is
  begin
    return x;
  end function;
  signal s : bit_vector(15 downto 0);
  constant w : positive := iw(g);
  alias lo is s(w - 1 downto 0);
  alias bit_w is s(w - 1);
begin
  assert lo'length = 8 and bit_w = '0';
end architecture;
