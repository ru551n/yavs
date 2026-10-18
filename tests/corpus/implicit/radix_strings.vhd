-- difftest: top=radix_strings std=2008
-- Implicit TO_STRING, TO_HSTRING and TO_OSTRING (LRM 2008 5.3.2.4) through sem, lowering
-- and the interpreter's implicit natives (#163).
entity radix_strings is end entity;

architecture test of radix_strings is
begin
  process
    variable b : bit_vector(7 downto 0) := x"a5";
    variable c : bit_vector(1 to 5) := "10011";
    variable s : string(1 to 3) := "abc";
  begin
    report to_string(b) & " " & to_hstring(b) & " " & to_ostring(b);
    report to_string(c) & " " & to_hstring(c) & " " & to_ostring(c);
    report to_string(42) & " " & to_string(true) & " " & to_string('x') & " " & to_string(10 ns);
    assert to_hstring(b) = "A5" severity failure;
    assert to_ostring(c) = "23" severity failure;
    report s & " " & to_string(s'length);
    wait;
  end process;
end architecture;
