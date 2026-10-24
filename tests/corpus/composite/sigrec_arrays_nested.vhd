-- difftest: top=sigrec_arrays_nested std=2008 stop=20ns
-- Nested shapes of record signals (#677): a two-dimensional array of records, an array of arrays
-- of records (a row, an element of a row), records with unconstrained array fields whose bounds
-- come from the signal, and a descending array of records with a logic vector field, each read
-- and driven whole and by parts with run-time indices.
library ieee; use ieee.std_logic_1164.all;
package sigrec_nested_pkg is
  type pix is record
    v : std_logic_vector(2 downto 0);
    w : std_logic;
    n : integer range 0 to 255;
  end record;
  type grid is array (0 to 1, 0 to 2) of pix;
  type row is array (0 to 2) of pix;
  type rows is array (1 to 2) of row;
  type cube is array (0 to 1) of rows;
  type buf is record
    d : std_logic_vector;
    k : integer;
  end record;
  type bufs is array (natural range <>) of buf;
  function mkp (i : integer) return pix;
  function img (p : pix) return string;
  function img (r : row) return string;
end package;
package body sigrec_nested_pkg is
  function mkp (i : integer) return pix is
    variable p : pix;
  begin
    for b in 0 to 2 loop
      p.v(b) := '1' when ((i / (2 ** b)) mod 2) = 1 else '0';
    end loop;
    p.w := '1' when i mod 2 = 0 else '0';
    p.n := i mod 256;
    return p;
  end;
  function img (p : pix) return string is
  begin
    return to_string(p.v) & std_logic'image(p.w) & integer'image(p.n);
  end;
  function img (r : row) return string is
  begin
    return img(r(0)) & "," & img(r(1)) & "," & img(r(2));
  end;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.sigrec_nested_pkg.all;
entity sigrec_arrays_nested is end;
architecture a of sigrec_arrays_nested is
  signal gr : grid;
  signal rs : rows;
  signal cu : cube;
  signal bs : bufs(3 downto 1)(d(5 downto 0));
  signal bs2 : bufs(0 to 1)(d(1 to 4));
begin
  ctl : process
    variable g : grid;
    variable r : row;
    variable b : bufs(3 downto 1)(d(5 downto 0));
  begin
    for i in 0 to 1 loop
      for j in 0 to 2 loop
        g(i, j) := mkp(10 * i + j);
      end loop;
    end loop;
    gr <= g;
    for i in 1 to 2 loop
      for j in 0 to 2 loop
        r(j) := mkp(30 + 10 * i + j);
      end loop;
      rs(i) <= r;
    end loop;
    for i in b'range loop
      b(i).d := (others => '1');
      b(i).d(i) := '0';
      b(i).k := i * i;
    end loop;
    bs <= b;
    bs2(0) <= (d => "1010", k => 5);
    bs2(1) <= (d => "0110", k => 6);
    for i in 0 to 1 loop
      cu(i) <= (rs'range => (0 to 2 => mkp(200 + i)));
    end loop;
    wait for 1 ns;
    -- Elements of the grid and of the rows by run-time indices.
    for i in 0 to 1 loop
      for j in 0 to 2 loop
        gr(i, j) <= mkp(100 + 10 * i + j);
      end loop;
    end loop;
    for i in 1 to 2 loop
      for j in 0 to 2 loop
        rs(i)(j) <= mkp(150 + 10 * i + j);
      end loop;
    end loop;
    for i in bs'range loop
      bs(i) <= (d => not b(i).d, k => -i);
    end loop;
    wait for 1 ns;
    r := rs(2);
    r(1) := mkp(7);
    rs(1) <= r;
    bs(2).d(1 downto 0) <= "10";
    wait;
  end process;

  chk : process
    variable g : grid;
    variable r : row;
    variable b : bufs(3 downto 1)(d(5 downto 0));
    variable b2 : bufs(0 to 1)(d(1 to 4));
    variable rr : rows;
  begin
    for step in 1 to 6 loop
      wait for 0.5 ns;
      g := gr;
      report integer'image(step) & " gr=" & img(g(0, 0)) & " " & img(g(0, 2)) & " " & img(g(1, 1)) & " " & img(gr(1, 2));
      r := rs(1);
      report integer'image(step) & " rs1=" & img(r) & " rs2=" & img(rs(2));
      report integer'image(step) & " rs12=" & img(rs(1)(2)) & " " & img(rs(2)(0));
      rr := cu(1);
      report integer'image(step) & " cu=" & img(rr(1)) & " " & img(rr(2)) & " " & img(cu(0)(2)) & " " & img(cu(1)(1)(0));
      b := bs;
      b2 := bs2;
      for i in b'range loop
        report integer'image(step) & " b" & integer'image(i) & "=" & to_string(b(i).d) & "/" & integer'image(b(i).k);
      end loop;
      report integer'image(step) & " b2=" & to_string(b2(0).d) & "/" & integer'image(b2(0).k) & " " & to_string(bs2(1).d) & "/" & integer'image(bs2(1).k);
    end loop;
    wait;
  end process;
end;
