-- difftest: top=shared_elab_call std=2008 relaxed
-- A function that reads and writes a non-protected (VHDL-93 style) shared variable of its
-- region is called during elaboration (constants, generic map actuals, generate bounds): the
-- calls see each other's effects in order and the simulation starts with the result (#524).
entity sub is generic (g : natural); end entity;
architecture a of sub is begin
  process begin report "g = " & natural'image(g); wait; end process;
end architecture;

entity shared_elab_call is end entity;
architecture a of shared_elab_call is
  type rec is record n : natural; tag : bit_vector(1 downto 0); end record;
  type arr is array (0 to 3) of natural;
  shared variable cnt : natural := 0;
  shared variable r : rec := (n => 10, tag => "01");
  shared variable a : arr := (others => 100);
  impure function next_value return natural is
  begin
    cnt := cnt + 1;
    return cnt;
  end function;
  impure function next_rec return natural is
  begin
    r.n := r.n + 1;
    r.tag := not r.tag;
    a(r.n mod 4) := a(r.n mod 4) + 1;
    return r.n;
  end function;
  constant c1 : natural := next_value;
  constant c2 : natural := next_value;
  constant r1 : natural := next_rec;
  constant r2 : natural := next_rec;
begin
  u1 : entity work.sub generic map (g => c1);
  u2 : entity work.sub generic map (g => next_value);
  u3 : entity work.sub generic map (g => c2 * 10 + r1);
  b : block
    constant d : natural := next_value;
  begin
    process begin
      wait for 1 ns;
      report "cnt = " & natural'image(cnt) & " d = " & natural'image(d) & " r = " & natural'image(r.n)
        & " " & to_string(r.tag) & " a = " & natural'image(a(3)) & " " & natural'image(a(0))
        & " r2 = " & natural'image(r2);
      wait;
    end process;
  end block;
end architecture;
