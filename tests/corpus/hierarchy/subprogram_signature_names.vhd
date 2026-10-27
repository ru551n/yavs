-- difftest: top=subprogram_signature_names std=2008 ref=hand
-- ref=hand: the reference simulator output with its library name `lib` read as `work` (the library the harness analyzes into for yavs).
-- 'instance_name and 'path_name of an object in a subprogram (16.2.5): the signature lists the
-- parameter type marks as declared, separated by a comma and a space, then `return` and the
-- result type mark (nothing before it when there are no parameters); nested subprograms and
-- operators extend the path. (#784)
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package sigpk is
  type pv is array (natural range <>) of positive;
  subtype byte is std_logic_vector(7 downto 0);
  type rec is record a : integer; end record;
  function f(x : std_logic; n : natural; b : byte; u : unsigned; r : rec) return std_logic_vector;
  impure function g return integer;
  procedure p(signal s : inout std_logic; variable v : out integer; i : in positive := 1);
  procedure outer(x : integer);
end package;
package body sigpk is
  function f(x : std_logic; n : natural; b : byte; u : unsigned; r : rec)
    return std_logic_vector is
    variable v : pv(0 to 1);
  begin
    report "F " & v'instance_name & " | " & v'path_name;
    return b;
  end function;
  impure function g return integer is
    variable v : pv(0 to 1);
  begin
    report "G " & v'instance_name;
    return 1;
  end function;
  procedure p(signal s : inout std_logic; variable v : out integer; i : in positive := 1) is
    variable w : pv(0 to 1);
  begin
    report "P " & w'instance_name & " | " & w'path_name;
    v := 0;
  end procedure;
  procedure outer(x : integer) is
    procedure inner(y : real; z : time) is
      variable w : pv(0 to 1);
    begin
      report "I " & w'instance_name & " | " & w'path_name;
    end procedure;
    function "+"(a, b : bit) return bit is
      variable w : pv(0 to 1);
    begin
      report "OP " & w'instance_name;
      return a;
    end function;
    variable bb : bit;
  begin
    inner(1.0, 1 ns);
    bb := '1' + '0';
  end procedure;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.sigpk.all;
entity subprogram_signature_names is end entity;
architecture t of subprogram_signature_names is
  signal s : std_logic;
begin
  process
    variable i : integer;
    variable sl : std_logic_vector(7 downto 0);
  begin
    sl := f('0', 1, x"00", "01", (a => 1));
    i := g;
    p(s, i);
    outer(1);
    wait;
  end process;
end architecture;
