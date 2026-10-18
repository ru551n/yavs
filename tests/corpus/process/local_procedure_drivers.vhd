-- difftest: top=tb_local_procedure_drivers std=2008
-- Signals assigned in procedures declared inside a process are driven by that process
-- (LRM 14.7.2): whole signals, indexed and record targets, package signals, nested
-- procedures, out signal parameters passed on from a local procedure, and two processes
-- whose local procedures drive one resolved signal.
library ieee;
use ieee.std_logic_1164.all;

package pkg is
  signal pkg_sig : integer := 0;
end package;

use work.pkg.all;
library ieee;
use ieee.std_logic_1164.all;

entity tb_local_procedure_drivers is end entity;

architecture a of tb_local_procedure_drivers is
  type rec is record
    a : integer;
    b : bit;
  end record;
  signal s   : integer := 0;
  signal v   : bit_vector(0 to 3) := "0000";
  signal r   : rec := (0, '0');
  signal o   : integer := 0;
  signal res : std_logic := 'Z';
  signal go  : boolean := false;

  procedure put(signal x : out integer; n : integer) is
  begin
    x <= n;
  end procedure;
begin
  main : process
    procedure setup is
    begin
      s <= 2;
      v(1) <= '1';
      r.b <= '1';
      pkg_sig <= 7;
    end procedure;

    procedure outer(n : integer) is
      procedure inner is
      begin
        s <= n;
        v(3) <= '1' after 1 ns;
      end procedure;
    begin
      inner;
      put(o, n + 1);
    end procedure;
  begin
    setup;
    wait for 1 ns;
    assert s = 2 report "s /= 2" severity failure;
    assert v = "0100" report "v(1)" severity failure;
    assert r = (0, '1') report "r.b" severity failure;
    assert pkg_sig = 7 report "pkg_sig" severity failure;
    report "setup ok";
    outer(5);
    wait for 0 ns;
    assert s = 5 report "s /= 5" severity failure;
    assert o = 6 report "o /= 6" severity failure;
    assert v = "0100" report "v(3) early" severity failure;
    wait for 1 ns;
    assert v = "0101" report "v(3)" severity failure;
    report "outer ok";
    go <= true;
    wait for 1 ns;
    assert res = 'X' report "res not X: " & std_logic'image(res) severity failure;
    wait for 1 ns;
    assert res = '1' report "res not 1: " & std_logic'image(res) severity failure;
    report "resolved ok";
    wait;
  end process;

  drv0 : process
    procedure drive(x : std_logic) is
    begin
      res <= x;
    end procedure;
  begin
    wait until go;
    drive('0');
    wait for 1 ns;
    drive('Z');
    wait;
  end process;

  drv1 : process
    procedure drive(x : std_logic) is
    begin
      res <= x;
    end procedure;
  begin
    wait until go;
    drive('1');
    wait;
  end process;
end architecture;
