-- difftest: top=tb_proc_local_drivers std=2008
-- Signals driven from procedures declared in a process: indices, slices and
-- constants computed from parameters are evaluated when the call runs (#205),
-- and the driver belongs to the process (#206).
entity tb_proc_local_drivers is end;
architecture t of tb_proc_local_drivers is
  type rec_t is record
    tag : integer;
    arr : bit_vector(0 to 3);
  end record;
  signal flags : bit_vector(1 to 4) := "0000";
  signal vec : bit_vector(7 downto 0) := x"00";
  signal s : integer := 0;
  signal ints : integer_vector(0 to 3) := (others => 0);
  signal r : rec_t := (0, "0000");
begin
  process
    procedure set(i : positive) is
    begin
      flags(i) <= '1';
    end procedure;
    procedure setup is
    begin
      s <= 2;
    end procedure;
    procedure put(i : natural; v : integer) is
      constant k : natural := 3 - i;
      procedure inner(j : natural) is
      begin
        ints(j) <= v * 10;
      end procedure;
    begin
      ints(k) <= v;
      inner(i);
    end procedure;
    procedure fill(hi, lo : natural; b : bit) is
    begin
      vec(hi downto lo) <= (hi downto lo => b);
    end procedure;
    procedure mark(i : natural) is
    begin
      r.arr <= "0100";
      r.tag <= i;
    end procedure;
  begin
    set(2);
    setup;
    put(0, 7);
    fill(5, 2, '1');
    mark(1);
    wait for 1 ns;
    report "flags=" & to_string(flags) & " s=" & integer'image(s);
    report "ints=" & integer'image(ints(0)) & "," & integer'image(ints(1)) & ","
      & integer'image(ints(2)) & "," & integer'image(ints(3));
    report "vec=" & to_string(vec) & " r=" & integer'image(r.tag) & "," & to_string(r.arr);
    assert flags = "0100" and s = 2 severity failure;
    assert ints = (70, 0, 0, 7) severity failure;
    assert vec = "00111100" and r = (1, "0100") severity failure;
    for n in 1 to 4 loop
      set(n);
      wait for 1 ns;
    end loop;
    fill(7, 6, '1');
    fill(3, 0, '0');
    put(2, 5);
    wait for 1 ns;
    report "flags=" & to_string(flags) & " vec=" & to_string(vec) & " ints(1)="
      & integer'image(ints(1)) & " ints(2)=" & integer'image(ints(2));
    assert flags = "1111" and vec = "11110000" severity failure;
    assert ints = (70, 5, 50, 7) severity failure;
    wait;
  end process;
end;
