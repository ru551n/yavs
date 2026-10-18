-- difftest: top=aggregate_open_elements std=2008
-- 2008 array-valued aggregate elements of arrays whose element subtype the type leaves
-- open (#230): the element constraint comes from the context or the first value, for
-- logic and integer rows, positional and named.
entity aggregate_open_elements is end;
architecture t of aggregate_open_elements is
  type wv is array (natural range <>) of bit_vector;
  type iv is array (natural range <>) of integer;
  type ivv is array (natural range <>) of iv;
  function img(v : iv) return string is
  begin
    if v'length = 0 then return ""; end if;
    return integer'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end;
  function rows(m : wv) return string is
  begin
    if m'length = 0 then return ""; end if;
    return to_string(m(m'left)) & " " & rows(m(m'left + 1 to m'right));
  end;
  function show(m : wv) return string is
  begin
    return rows(m) & integer'image(m'left) & ":" & integer'image(m'right) & ":"
      & integer'image(m(m'left)'left);
  end;
begin
  process
    variable r2 : wv(0 to 1)(3 downto 0) := ("0001", "0010");
    variable r1 : wv(0 to 0)(3 downto 0) := (0 => "1111");
    variable m : wv(0 to 3)(3 downto 0);
    variable q2 : ivv(0 to 1)(1 to 2) := ((1, 2), (3, 4));
    variable q : ivv(0 to 2)(1 to 2);
  begin
    m := (r2, r1, "0100");
    report show(m);
    report show(wv'(r1, r2));
    m := (2 to 3 => r2, others => "1010");
    report show(m);
    report show(wv'(1 to 2 => r2, 3 => "1100"));
    q := (q2, (5, 6));
    report img(q(0)) & " " & img(q(1)) & " " & img(q(2));
    q := (1 to 2 => q2, 0 => (7, 8));
    report img(q(0)) & " " & img(q(1)) & " " & img(q(2));
    assert m(3) = "0010" and q(2)(2) = 4 severity failure;
    wait;
  end process;
end;
