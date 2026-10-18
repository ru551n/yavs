-- yavs-test: std=2008
-- expect: none
-- LRM 8.1, 11.8, 14.5.3 (a generate parameter is globally static; a source in a generate exists only if elaborated)
entity e is end entity;
architecture a of e is
  signal s : bit_vector(0 to 3);
  signal t, u : bit;
begin
  g: for i in 0 to 1 generate
    p1: process begin s(i) <= '1'; wait; end process;
  end generate;
  p2: process begin s(3) <= '1'; wait; end process;
  h: if true generate t <= '1'; else generate t <= '0'; end generate;
  h1: if true generate u <= '1'; end generate;
  h2: if false generate u <= '0'; end generate;
end architecture;
