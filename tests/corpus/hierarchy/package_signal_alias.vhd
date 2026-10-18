-- difftest: top=tb_package_signal_alias
-- Signal aliases declared in a package (LRM 6.6.2), driven and read from a design and
-- passed as signal actuals.
package psa_pkg is
  type arr_t is array (natural range <>) of integer;
  signal s : arr_t(0 to 3) := (others => 0);
  signal b : boolean := false;
  alias s_tail is s(2 to 3);
  alias s_up : arr_t(1 to 2) is s(2 to 3);
  alias flag is b;
  procedure poke(signal x : out integer; v : integer);
end package;

package body psa_pkg is
  procedure poke(signal x : out integer; v : integer) is
  begin
    x <= v;
  end procedure;
end package body;

use work.psa_pkg.all;

entity tb_package_signal_alias is end entity;

architecture a of tb_package_signal_alias is
begin
  p1 : process
  begin
    s_tail(2) <= 5;
    poke(s_up(2), 6);
    flag <= true;
    wait for 1 ns;
    assert s_tail(2) = 5 and s(3) = 6 report "bad " & integer'image(s(3)) severity failure;
    s_up <= (1, 2);
    wait;
  end process;

  p2 : process
  begin
    wait until flag;
    report "flag event=" & boolean'image(flag'event);
    wait on s_up;
    report "s_up=" & integer'image(s_up(1)) & integer'image(s_up(2));
    report "ok";
    wait;
  end process;
end architecture;
