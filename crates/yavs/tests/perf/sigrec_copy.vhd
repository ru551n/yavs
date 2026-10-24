-- A combinational process with a sensitivity list reads a wide record signal as a whole several
-- times per activation (as the argument of functions, in branches, in a loop): the process keeps
-- one copy of the signal and fills it once per activation. Every result is also computed from
-- single fields, which are read leaf by leaf and never from the copy, and compared.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity sigrec_copy is end;
architecture a of sigrec_copy is
  type inner is record
    p : std_logic;
    q : std_logic_vector(3 downto 0);
    n : natural range 0 to 7;
    t : std_logic;
  end record;
  type rec is record
    a : unsigned(15 downto 0);
    i1 : inner;
    i2 : inner;
    b : std_logic_vector(7 downto 0);
    c : std_logic;
    d : natural range 0 to 1000;
    e : std_logic_vector(3 downto 0);
    f : std_logic;
  end record;
  function fa (x : rec) return integer is begin return to_integer(x.a); end;
  function fb (x : rec) return integer is begin return to_integer(unsigned(x.b)) + x.d; end;
  function fi (x : rec) return integer is
    variable s : integer := x.i1.n + 2 * x.i2.n;
  begin
    if x.i1.p = '1' then s := s + to_integer(unsigned(x.i1.q)); end if;
    if x.i2.t = '1' then s := s + 3 * to_integer(unsigned(x.i2.q)); end if;
    return s;
  end;
  function fc (x : rec) return integer is
  begin
    if x.c = '1' then return to_integer(unsigned(x.e)); else return -1; end if;
  end;
  -- The same value from single fields, which the process reads leaf by leaf.
  function expect (a : unsigned; b : std_logic_vector; c : std_logic; d : natural; e : std_logic_vector; f : std_logic;
                   p1 : std_logic; q1 : std_logic_vector; n1 : natural; t2 : std_logic; q2 : std_logic_vector; n2 : natural) return integer is
    variable s, ec : integer;
  begin
    if c = '1' then ec := to_integer(unsigned(e)); else ec := -1; end if;
    s := to_integer(a) + (to_integer(unsigned(b)) + d) + ec;
    if f = '1' then s := s + to_integer(a); else s := s - (to_integer(unsigned(b)) + d); end if;
    s := s + 6 * ec + n1 + 2 * n2;
    if p1 = '1' then s := s + to_integer(unsigned(q1)); end if;
    if t2 = '1' then s := s + 3 * to_integer(unsigned(q2)); end if;
    return s;
  end;
  signal clk : std_logic := '0';
  signal r, rin : rec := (a => (others => '0'), i1 => ('1', "0011", 2, '0'), i2 => ('0', "0101", 3, '1'),
                                  b => x"01", c => '0', d => 1, e => "0001", f => '0');
  signal sum, want, errors : integer := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  comb : process (r)
    variable v : rec;
    variable s : integer;
  begin
    v := r;
    s := fa(r) + fb(r) + fc(r) + fi(r);
    if r.f = '1' then
      s := s + fa(r);
    else
      s := s - fb(r);
    end if;
    for i in 1 to 3 loop
      s := s + fc(r) * i;
    end loop;
    v.a := r.a + 3;
    v.b := std_logic_vector(unsigned(r.b) + 5);
    v.c := not r.c;
    v.d := (r.d * 7 + 1) mod 1000;
    v.e := std_logic_vector(unsigned(r.e) + 1);
    v.f := r.f xor r.c;
    v.i1.p := not r.i1.p;
    v.i1.q := std_logic_vector(unsigned(r.i1.q) + 1);
    v.i1.n := (r.i1.n + 1) mod 8;
    v.i1.t := r.i1.t xor r.i2.p;
    v.i2.p := r.i1.t;
    v.i2.q := std_logic_vector(unsigned(r.i2.q) + 3);
    v.i2.n := (r.i2.n + 5) mod 8;
    v.i2.t := not r.i2.t;
    rin <= v;
    sum <= s;
    want <= expect(r.a, r.b, r.c, r.d, r.e, r.f, r.i1.p, r.i1.q, r.i1.n, r.i2.t, r.i2.q, r.i2.n);
  end process;

  process (clk) begin
    if rising_edge(clk) then r <= rin; end if;
  end process;

  process (clk)
    variable n : natural := 0;
  begin
    if falling_edge(clk) then
      if sum /= want then errors <= errors + 1; end if;
      n := n + 1;
      if n = 100 then done <= true; end if;
    end if;
  end process;

  process begin
    wait until done;
    report "sum=" & integer'image(sum) & " want=" & integer'image(want) & " errors=" & integer'image(errors) & " a=" & integer'image(to_integer(r.a));
    wait;
  end process;
end;
