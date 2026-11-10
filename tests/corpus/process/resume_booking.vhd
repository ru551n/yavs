-- difftest: top=resume_booking
-- A process that waits inside a procedure and does its work after the procedure returns, as a
-- bus procedure that waits for a handshake and the test sequence after it. Each activation resumes
-- inside `wait_toggle`; under `--tier=auto` the return to the process names the process for the
-- sampler again (`booked_returns`); every tier must print the same.
library ieee;
use ieee.std_logic_1164.all;

package resume_booking_pkg is
  procedure wait_toggle(signal s : in std_logic);
end package;

package body resume_booking_pkg is
  procedure wait_toggle(signal s : in std_logic) is
  begin
    wait on s;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.resume_booking_pkg.all;

entity resume_booking is
end entity;

architecture a of resume_booking is
  signal s : std_logic := '0';
begin
  toggle : process
  begin
    for i in 1 to 500 loop
      wait for 1 ns;
      s <= not s;
    end loop;
    wait;
  end process;

  waiter : process
    variable woke : natural := 0;
    variable acc : integer := 1;
  begin
    while woke < 500 loop
      wait_toggle(s);
      acc := (acc * 3 + 0) mod 1000003;
      acc := (acc * 4 + 1) mod 1000003;
      acc := (acc * 5 + 2) mod 1000003;
      acc := (acc * 6 + 3) mod 1000003;
      acc := (acc * 7 + 4) mod 1000003;
      acc := (acc * 8 + 5) mod 1000003;
      acc := (acc * 9 + 6) mod 1000003;
      acc := (acc * 10 + 7) mod 1000003;
      acc := (acc * 11 + 8) mod 1000003;
      acc := (acc * 12 + 9) mod 1000003;
      acc := (acc * 13 + 10) mod 1000003;
      acc := (acc * 3 + 11) mod 1000003;
      acc := (acc * 4 + 12) mod 1000003;
      acc := (acc * 5 + 13) mod 1000003;
      acc := (acc * 6 + 14) mod 1000003;
      acc := (acc * 7 + 15) mod 1000003;
      acc := (acc * 8 + 16) mod 1000003;
      acc := (acc * 9 + 17) mod 1000003;
      acc := (acc * 10 + 18) mod 1000003;
      acc := (acc * 11 + 19) mod 1000003;
      acc := (acc * 12 + 20) mod 1000003;
      acc := (acc * 13 + 21) mod 1000003;
      acc := (acc * 3 + 22) mod 1000003;
      acc := (acc * 4 + 23) mod 1000003;
      acc := (acc * 5 + 24) mod 1000003;
      acc := (acc * 6 + 25) mod 1000003;
      acc := (acc * 7 + 26) mod 1000003;
      acc := (acc * 8 + 27) mod 1000003;
      acc := (acc * 9 + 28) mod 1000003;
      acc := (acc * 10 + 29) mod 1000003;
      acc := (acc * 11 + 30) mod 1000003;
      acc := (acc * 12 + 31) mod 1000003;
      acc := (acc * 13 + 32) mod 1000003;
      acc := (acc * 3 + 33) mod 1000003;
      acc := (acc * 4 + 34) mod 1000003;
      acc := (acc * 5 + 35) mod 1000003;
      acc := (acc * 6 + 36) mod 1000003;
      acc := (acc * 7 + 37) mod 1000003;
      acc := (acc * 8 + 38) mod 1000003;
      acc := (acc * 9 + 39) mod 1000003;
      acc := (acc * 10 + 40) mod 1000003;
      acc := (acc * 11 + 41) mod 1000003;
      acc := (acc * 12 + 42) mod 1000003;
      acc := (acc * 13 + 43) mod 1000003;
      acc := (acc * 3 + 44) mod 1000003;
      acc := (acc * 4 + 45) mod 1000003;
      acc := (acc * 5 + 46) mod 1000003;
      acc := (acc * 6 + 47) mod 1000003;
      acc := (acc * 7 + 48) mod 1000003;
      acc := (acc * 8 + 49) mod 1000003;
      acc := (acc * 9 + 50) mod 1000003;
      acc := (acc * 10 + 51) mod 1000003;
      acc := (acc * 11 + 52) mod 1000003;
      acc := (acc * 12 + 53) mod 1000003;
      acc := (acc * 13 + 54) mod 1000003;
      acc := (acc * 3 + 55) mod 1000003;
      acc := (acc * 4 + 56) mod 1000003;
      acc := (acc * 5 + 57) mod 1000003;
      acc := (acc * 6 + 58) mod 1000003;
      acc := (acc * 7 + 59) mod 1000003;
      acc := (acc * 8 + 60) mod 1000003;
      acc := (acc * 9 + 61) mod 1000003;
      acc := (acc * 10 + 62) mod 1000003;
      acc := (acc * 11 + 63) mod 1000003;
      acc := (acc * 12 + 64) mod 1000003;
      acc := (acc * 13 + 65) mod 1000003;
      acc := (acc * 3 + 66) mod 1000003;
      acc := (acc * 4 + 67) mod 1000003;
      acc := (acc * 5 + 68) mod 1000003;
      acc := (acc * 6 + 69) mod 1000003;
      acc := (acc * 7 + 70) mod 1000003;
      acc := (acc * 8 + 71) mod 1000003;
      acc := (acc * 9 + 72) mod 1000003;
      acc := (acc * 10 + 73) mod 1000003;
      acc := (acc * 11 + 74) mod 1000003;
      acc := (acc * 12 + 75) mod 1000003;
      acc := (acc * 13 + 76) mod 1000003;
      acc := (acc * 3 + 77) mod 1000003;
      acc := (acc * 4 + 78) mod 1000003;
      acc := (acc * 5 + 79) mod 1000003;
      acc := (acc * 6 + 80) mod 1000003;
      acc := (acc * 7 + 81) mod 1000003;
      acc := (acc * 8 + 82) mod 1000003;
      acc := (acc * 9 + 83) mod 1000003;
      acc := (acc * 10 + 84) mod 1000003;
      acc := (acc * 11 + 85) mod 1000003;
      acc := (acc * 12 + 86) mod 1000003;
      acc := (acc * 13 + 87) mod 1000003;
      acc := (acc * 3 + 88) mod 1000003;
      acc := (acc * 4 + 89) mod 1000003;
      acc := (acc * 5 + 90) mod 1000003;
      acc := (acc * 6 + 91) mod 1000003;
      acc := (acc * 7 + 92) mod 1000003;
      acc := (acc * 8 + 93) mod 1000003;
      acc := (acc * 9 + 94) mod 1000003;
      acc := (acc * 10 + 95) mod 1000003;
      acc := (acc * 11 + 96) mod 1000003;
      acc := (acc * 12 + 97) mod 1000003;
      acc := (acc * 13 + 98) mod 1000003;
      acc := (acc * 3 + 99) mod 1000003;
      woke := woke + 1;
    end loop;
    report "acc = " & integer'image(acc);
    assert acc = 977261 report "wrong acc" severity failure;
    wait;
  end process;
end architecture;
