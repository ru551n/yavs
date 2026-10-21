-- yavs-test: std=2019
-- expect: E1311 12:58 in port without a default
-- LRM 6.5.7.1 (2019): an open sub-element of a port of mode in needs a default, as an open port does (sim1 agrees)
entity e is
end entity;
architecture a of e is
  signal v : bit_vector(1 downto 0);
  component c is
    port (ip : in bit_vector(3 downto 0));
  end component;
begin
  u : c port map (ip(1 downto 0) => v, ip(3 downto 2) => open);
end architecture;
