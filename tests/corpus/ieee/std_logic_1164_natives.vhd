-- difftest: top=std_logic_1164_natives std=2008
-- std_logic_1164 gates of a vector and a scalar (every pair of the nine values, both sides,
-- lengths around the 64-element blocks), to_stdulogicvector and to_stdlogicvector of bit_vector,
-- and case? on std_ulogic_vector with '-' in a choice and metavalues (not '-', an error in the
-- expression, LRM 10.9) in the expression, each
-- against an element loop or the scalar ?=.
library ieee;
use ieee.std_logic_1164.all;
entity std_logic_1164_natives is end entity;
architecture a of std_logic_1164_natives is
  type widths_t is array (natural range <>) of natural;
  constant widths : widths_t := (0, 1, 9, 63, 64, 65, 130);
  constant codes : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
begin
  process
    variable acc, cases, bad : natural := 0;
    variable seed : natural := 777;

    impure function rnd(n : positive) return natural is
    begin
      seed := (seed * 75 + 74) mod 65537;
      return seed mod n;
    end function;

    procedure ok(c : boolean; what : string) is
    begin
      cases := cases + 1;
      if not c then
        bad := bad + 1;
        report "mismatch: " & what severity error;
      end if;
    end procedure;

    procedure mix(v : natural) is
    begin
      acc := (acc * 31 + v) mod 1000003;
    end procedure;

    procedure mix(x : std_ulogic_vector) is
    begin
      for i in x'range loop mix(std_ulogic'pos(x(i))); end loop;
      mix(x'length);
    end procedure;

    procedure gates(n : natural) is
      variable v : std_ulogic_vector(n + 9 downto 10);
      variable r : std_ulogic_vector(1 to n);
      variable s : std_ulogic;
      alias vv : std_ulogic_vector(1 to n) is v;
    begin
      for i in v'range loop v(i) := codes((i * 7 + rnd(9)) mod 9); end loop;
      for k in codes'range loop
        s := codes(k);
        for i in 1 to n loop r(i) := vv(i) and s; end loop;
        ok((v and s) = r, "and vs");
        for i in 1 to n loop r(i) := s nand vv(i); end loop;
        ok((s nand v) = r, "nand sv");
        for i in 1 to n loop r(i) := vv(i) or s; end loop;
        ok((v or s) = r, "or vs");
        for i in 1 to n loop r(i) := s nor vv(i); end loop;
        ok((s nor v) = r, "nor sv");
        for i in 1 to n loop r(i) := vv(i) xor s; end loop;
        ok((v xor s) = r, "xor vs");
        for i in 1 to n loop r(i) := s xnor vv(i); end loop;
        ok((s xnor v) = r, "xnor sv");
        mix(r);
        mix(v and s);
        mix(s or v);
      end loop;
    end procedure;

    procedure convs(n : natural) is
      variable b : bit_vector(3 to n + 2);
      variable u : std_ulogic_vector(n - 1 downto 0);
      alias bb : bit_vector(n - 1 downto 0) is b;
    begin
      for i in b'range loop b(i) := bit'val(rnd(2)); end loop;
      for i in u'range loop u(i) := '1' when bb(i) = '1' else '0'; end loop;
      ok(to_stdulogicvector(b) = u, "to_stdulogicvector");
      ok(to_stdlogicvector(b) = std_logic_vector(u), "to_stdlogicvector");
      mix(to_stdulogicvector(b));
    end procedure;

    variable k, want : integer;
    variable e : std_ulogic_vector(7 downto 0);
    variable e70 : std_ulogic_vector(69 downto 0);
  begin
    for wi in widths'range loop
      gates(widths(wi));
      convs(widths(wi));
    end loop;
    -- case?: '-' in a choice matches anything, a metavalue in the expression matches only '-'.
    for t in 0 to 299 loop
      for i in e'range loop
        e(i) := codes(rnd(8)) when rnd(4) = 0 else codes(2 + rnd(2));
      end loop;
      case? e is
        when "1-------" => k := 0;
        when "0000----" => k := 1;
        when "01-0-1-0" => k := 2;
        when others => k := 3;
      end case?;
      if (e ?= "1-------") = '1' then want := 0;
      elsif (e ?= "0000----") = '1' then want := 1;
      elsif (e ?= "01-0-1-0") = '1' then want := 2;
      else want := 3;
      end if;
      ok(k = want, "case? 8");
      mix(k);
      for i in e70'range loop
        e70(i) := codes(rnd(8)) when rnd(16) = 0 else codes(2 + rnd(2));
      end loop;
      e70(69) := '0' when t mod 3 = 0 else e70(69);
      case? e70 is
        when (69 => '0', others => '-') => k := 0;
        when (69 => '1', 0 => '1', others => '-') => k := 1;
        when others => k := 2;
      end case?;
      if (e70(69) ?= '0') = '1' then want := 0;
      elsif (e70(69) ?= '1') = '1' and (e70(0) ?= '1') = '1' then want := 1;
      else want := 2;
      end if;
      ok(k = want, "case? 70");
      mix(k);
    end loop;
    report "std_logic_1164_natives cases=" & integer'image(cases) & " acc=" & integer'image(acc)
      & " bad=" & integer'image(bad);
    assert bad = 0 severity failure;
    wait;
  end process;
end architecture;
