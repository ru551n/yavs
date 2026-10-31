-- yavs-test: std=2008
-- expect: W1403 16:41 static value is outside subtype
-- expect: W1403 16:59 static value is outside subtype
-- LRM 9.4, 14.3: the instance may never be elaborated, so analysis only warns (#910)
package cfg is
  constant n : integer := 0;
end package;
entity e is
  generic (g : integer range 1 to 8 := 8);
  port (p : in integer range 1 to 8 := 1);
end entity;
use work.cfg.all;
entity t is end entity;
architecture a of t is begin
  gen : if n /= 0 generate
    u : entity work.e generic map (g => n) port map (p => n);
  end generate;
end architecture;
