-- String handling as logging and message code does it: image/to_string, concatenation,
-- functions returning unconstrained strings, case conversion, searching, 'value.
entity bench_strings_heavy is
  generic (N : natural := 20000);
end entity;

architecture sim of bench_strings_heavy is
  function to_upper(s : string) return string is
    variable r : string(s'range) := s;
  begin
    for i in r'range loop
      if r(i) >= 'a' and r(i) <= 'z' then
        r(i) := character'val(character'pos(r(i)) - 32);
      end if;
    end loop;
    return r;
  end function;

  function pad_left(s : string; w : natural) return string is
  begin
    if s'length >= w then
      return s;
    end if;
    return (1 to w - s'length => ' ') & s;
  end function;

  function find(s, pat : string) return natural is
  begin
    if pat'length > s'length then
      return 0;
    end if;
    for i in s'low to s'high - pat'length + 1 loop
      if s(i to i + pat'length - 1) = pat then
        return i;
      end if;
    end loop;
    return 0;
  end function;
begin
  process
    variable sum : natural := 0;
    variable v   : integer;
    variable slv : bit_vector(15 downto 0);
  begin
    for i in 1 to N loop
      for b in 0 to 15 loop
        slv(b) := bit'val((i / 2 ** b) mod 2);
      end loop;
      v := integer'value(pad_left(integer'image(i), 8));
      sum := (sum + v) mod 1000003;
      sum := sum + find(to_upper("msg " & to_string(i) & " from " & time'image(i * 1 ns) & " done"), "FROM");
      sum := sum + to_string(slv)'length + to_hstring(slv)'length;
      sum := sum + character'image('a')'length + boolean'image(sum > 5)'length;
      sum := sum + real'image(real(i) / 7.0)'length;
      sum := sum mod 1000003;
    end loop;
    report "strings_heavy sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
