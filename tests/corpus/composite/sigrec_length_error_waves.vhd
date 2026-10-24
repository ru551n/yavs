-- difftest: top=sigrec_length_error_waves std=2008 exit=1
-- The length check of a record value in a waveform of several elements, which a record signal
-- drives leaf by leaf (#677): the second element has another field length than the signal.
library ieee; use ieee.std_logic_1164.all;
entity sigrec_length_error_waves is end;
architecture a of sigrec_length_error_waves is
  type rec_t is record
    a : std_logic_vector;
    b : integer;
  end record;
  subtype r8 is rec_t(a(7 downto 0));
  subtype r4 is rec_t(a(3 downto 0));
  signal s : r8 := (a => x"00", b => 0);
begin
  p : process
    variable w : r4 := (a => "0101", b => 3);
    variable ok : r8 := (a => x"a5", b => 3);
  begin
    s <= ok after 1 ns, ok after 2 ns;
    wait for 3 ns;
    report "assigned " & to_string(s.a) & " " & integer'image(s.b);
    s <= ok after 1 ns, w after 2 ns;
    wait for 3 ns;
    report "not reached";
    wait;
  end process;
end architecture;
