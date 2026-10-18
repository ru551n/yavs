-- yavs-test: std=2019
-- expect: none
-- LRM 6.5.7.1 (2019), LCS-2016-001: sub-elements of a port can be associated with open (#178)
entity e is
end entity;
architecture a of e is
  signal v : bit_vector(15 downto 0);
  component c is
    port (lp : out bit_vector(31 downto 0); ip : in bit_vector(3 downto 0) := "0000");
  end component;
begin
  u : c port map (lp(15 downto 0) => v, lp(31 downto 16) => open, ip(1 downto 0) => v(1 downto 0), ip(3 downto 2) => open);
end architecture;
