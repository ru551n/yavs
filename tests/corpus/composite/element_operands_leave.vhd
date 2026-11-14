-- difftest: top=element_operands_leave std=2008
-- Element operands read in place where compiled code leaves in the middle (#1122): one statement of 64
-- element operands with a metavalue in every other round, so the native's rare way makes the temporaries and the
-- temporary area fills; the interpreter goes on from the first temporary of the operation that left. In a function
-- (a native frame) and in the process.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity element_operands_leave is
end entity;

architecture sim of element_operands_leave is
  type t_arr is array (natural range <>) of signed(15 downto 0);
  function sum64(v : t_arr) return signed is
  begin
    return v(0) + v(1) + v(2) + v(3) + v(4) + v(5) + v(6) + v(7) + v(8) + v(9) + v(10) + v(11) + v(12) + v(13) + v(14) + v(15) + v(16) + v(17) + v(18) + v(19) + v(20) + v(21) + v(22) + v(23) + v(24) + v(25) + v(26) + v(27) + v(28) + v(29) + v(30) + v(31) + v(32) + v(33) + v(34) + v(35) + v(36) + v(37) + v(38) + v(39) + v(40) + v(41) + v(42) + v(43) + v(44) + v(45) + v(46) + v(47) + v(48) + v(49) + v(50) + v(51) + v(52) + v(53) + v(54) + v(55) + v(56) + v(57) + v(58) + v(59) + v(60) + v(61) + v(62) + v(63);
  end function;
begin
  process is
    variable v : t_arr(0 to 63);
    variable s : signed(15 downto 0);
    variable n : integer := 0;
  begin
    for i in 0 to 23 loop
      for k in v'range loop
        v(k) := to_signed(i * 5 + k, 16);
      end loop;
      if i mod 2 = 0 then
        v(i mod 64)(3) := 'X';
      end if;
      s := sum64(v);
      report "f " & integer'image(i) & " " & to_hstring(std_logic_vector(s));
      s := v(0) + v(1) + v(2) + v(3) + v(4) + v(5) + v(6) + v(7) + v(8) + v(9) + v(10) + v(11) + v(12) + v(13) + v(14) + v(15) + v(16) + v(17) + v(18) + v(19) + v(20) + v(21) + v(22) + v(23) + v(24) + v(25) + v(26) + v(27) + v(28) + v(29) + v(30) + v(31) + v(32) + v(33) + v(34) + v(35) + v(36) + v(37) + v(38) + v(39) + v(40) + v(41) + v(42) + v(43) + v(44) + v(45) + v(46) + v(47) + v(48) + v(49) + v(50) + v(51) + v(52) + v(53) + v(54) + v(55) + v(56) + v(57) + v(58) + v(59) + v(60) + v(61) + v(62) + v(63);
      report "p " & integer'image(i) & " " & to_hstring(std_logic_vector(s));
    end loop;
    wait;
  end process;
end architecture;
