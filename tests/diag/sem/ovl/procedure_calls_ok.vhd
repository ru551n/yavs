-- yavs-test: std=2008
-- expect: none
-- LRM 4.2, 10.7, 11.4
package pk is
  procedure incr (variable x : inout integer; constant by : in integer := 1);
  procedure split (constant v : in bit_vector; variable hi, lo : out bit);
  procedure drive (signal s : out bit; constant v : in bit);
end package;
package body pk is
  procedure incr (variable x : inout integer; constant by : in integer := 1) is
  begin
    x := x + by;
  end procedure;
  procedure split (constant v : in bit_vector; variable hi, lo : out bit) is
  begin
    hi := v(v'left);
    lo := v(v'right);
  end procedure;
  procedure drive (signal s : out bit; constant v : in bit) is
  begin
    s <= v;
  end procedure;
end package body;
use work.pk.all;
entity e is end entity;
architecture a of e is
  signal s, t : bit;
  procedure local (n : integer) is
  begin
  end procedure;
begin
  process
    variable i : integer := 0;
    variable h, l : bit;
  begin
    incr(i);
    incr(i, 2);
    incr(by => 3, x => i);
    work.pk.incr(i);
    split("10", h, l);
    split(v => "01", lo => l, hi => h);
    drive(s, h);
    local(i);
    wait;
  end process;
  drive(t, '1');
end architecture;
