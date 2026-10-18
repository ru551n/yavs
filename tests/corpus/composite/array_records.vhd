-- difftest: top=array_records std=2008
-- Signals of arrays of records and records with array fields (#200): whole, element,
-- field, bit and slice targets, static and computed indices, element sensitivity, ports.
entity aor_sink is
  generic (n : positive);
  port (
    i : in  bit_vector(1 to n);
    o : out natural
  );
end;
architecture t of aor_sink is
begin
  process (i)
    variable c : natural;
  begin
    c := 0;
    for k in i'range loop
      if i(k) = '1' then c := c + 1; end if;
    end loop;
    o <= c;
  end process;
end;

package aor_pkg is
  type cell is record ok : boolean; n : natural; b : bit_vector(2 downto 0); end record;
  type cells is array (1 to 3) of cell;
  type bank is record id : character; cs : cells; end record;
end;

use work.aor_pkg.all;
entity aor_unit is
  port (
    a : in  cells;
    s : out cells
  );
end;
architecture t of aor_unit is
begin
  process (a) begin
    for k in a'range loop
      s(k) <= (not a(k).ok, a(k).n * 2, not a(k).b);
    end loop;
  end process;
end;

use work.aor_pkg.all;
entity array_records is end;
architecture t of array_records is
  signal cs : cells := ((true, 1, "001"), (false, 2, "010"), (true, 3, "100"));
  signal bk : bank := ('a', (others => (false, 0, "000")));
  type pair is record tag : character; bits : bit_vector(0 to 2); end record;
  signal pr : pair := ('p', "000");
  signal so : cells;
  signal cnt : natural;
  function img(c : cell) return string is
  begin
    return boolean'image(c.ok) & "/" & integer'image(c.n) & "/" & bit'image(c.b(2))
      & bit'image(c.b(1)) & bit'image(c.b(0));
  end;
  function img(v : cells) return string is
  begin
    return img(v(1)) & " " & img(v(2)) & " " & img(v(3));
  end;
begin
  u : entity work.aor_unit port map (a => cs, s => so);
  c : entity work.aor_sink generic map (3) port map (i => pr.bits, o => cnt);

  drv : process
    variable k : integer := 2;
  begin
    wait for 1 ns;
    cs <= ((false, 10, "111"), (true, 20, "000"), (false, 30, "101"));
    wait for 1 ns;
    cs(2) <= (false, 21, "011");
    wait for 1 ns;
    cs(3).n <= 31;
    cs(1).b(1) <= '0';
    wait for 1 ns;
    for j in 1 to 3 loop
      cs(j).n <= cs(j).n + j;
    end loop;
    cs(k).ok <= true;
    wait for 1 ns;
    cs(2 to 3) <= ((true, 7, "110"), (true, 8, "001"));
    bk.cs(2).n <= 5;
    bk.id <= 'z';
    pr.bits(1) <= '1';
    wait for 1 ns;
    bk.cs(k to 3) <= (others => (true, 9, "111"));
    pr.bits(2) <= '1';
    wait for 1 ns;
    cs(1).ok <= true;
    wait for 1 ns;
    assert cs(1) = (true, 11, "101") severity failure;
    assert cs(2) = (true, 7, "110") and cs(3) = (true, 8, "001") severity failure;
    assert bk = ('z', ((false, 0, "000"), (true, 9, "111"), (true, 9, "111"))) severity failure;
    assert so(2) = (false, 14, "001") severity failure;
    assert cnt = 2 severity failure;
    report "done";
    wait;
  end process;

  process (cs) begin report "cs " & img(cs); end process;
  process (cs(2)) begin report "cs(2) " & img(cs(2)); end process;
  process begin
    wait on cs(3).n;
    report "cs(3).n " & integer'image(cs(3).n);
  end process;
  process (so) begin report "so " & img(so); end process;
  process (bk) begin report "bk " & bk.id & " " & img(bk.cs); end process;
  process (pr) begin report "pr " & pr.tag & bit'image(pr.bits(0)) & bit'image(pr.bits(1)) & bit'image(pr.bits(2)); end process;
  process (cnt) begin report "cnt " & integer'image(cnt); end process;
  process begin
    wait until cs(1).ok;
    report "cs(1).ok rose" ;
    wait;
  end process;
end;
