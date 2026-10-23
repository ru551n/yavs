-- yavs-test: std=1993
-- expect: none
-- LRM 1993 4.3.2
-- Writes, the actual of an out formal and the attributes that do not read the value.
package defs is
  procedure po (signal x : out bit);
end package;
entity leaf is
  port (i : in bit := '0'; o : out bit);
end entity;
architecture a of leaf is begin o <= i; end architecture;
use work.defs.all;
entity e is
  port (p : out bit; q : out bit; r : out bit; w : out bit_vector(3 downto 0));
end entity;
architecture a of e is
  signal s : bit;
begin
  p <= '1';
  w(1) <= '1';
  w(3 downto 2) <= "10";
  (s, w(0)) <= bit_vector'("11");
  po(q);
  u : entity work.leaf port map (o => r);
  process
    variable n : integer;
    variable b : bit;
  begin
    for k in w'range loop n := k; end loop;
    n := w'length + w'left;
    b := p'driving_value;
    wait;
  end process;
end architecture;
