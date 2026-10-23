-- yavs-test: std=2008
-- expect: none
-- LRM 7.4 (a guarded port of the entity or of a block is a signal of that declarative part; #644)
package pk is
  function wired_and (sig : bit_vector) return bit;
end package;
package body pk is
  function wired_and (sig : bit_vector) return bit is
  begin
    return '0';
  end function;
end package body;
use work.pk.all;
entity e is
  port (signal a : out wired_and bit bus);
  disconnect a : bit after 100 ns;
end entity;
architecture x of e is
begin
  b : block
    port (signal p : out wired_and bit bus);
    port map (p => open);
    disconnect p : bit after 5 ns;
  begin
  end block;
end architecture;
