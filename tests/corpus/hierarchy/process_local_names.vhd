-- difftest: top=process_local_names std=2008
-- 'path_name and 'instance_name of objects declared in processes and subprograms (LRM 16.2.5):
-- the process label, an empty element for an unlabeled process, subprograms with their profile.
entity process_local_names is end;
architecture a of process_local_names is
  function twice(x : integer; b : boolean) return integer is
    variable fv : integer := x;
  begin
    report "arch fn " & fv'path_name & " | " & fv'instance_name;
    return 2 * fv;
  end function;
begin
  main : process
    variable child : integer;
    constant k : integer := 3;
    procedure pr(x : integer) is
      variable pv : integer;
      procedure inner is
        variable iv : integer;
      begin
        report "inner " & iv'path_name & " | " & iv'instance_name;
      end procedure;
    begin
      assert pv'path_name = ":process_local_names:main:pr[integer]:pv";
      report "pr " & pv'path_name & " | " & pv'instance_name;
      report "prx " & x'path_name & " | " & x'instance_name;
      report "pr self " & pr'path_name & " | " & pr'instance_name;
      inner;
    end procedure;
    function fn(y : integer) return integer is
      variable fv : integer := 0;
    begin
      report "fn " & fv'path_name & " | " & fv'instance_name;
      return y;
    end function;
    variable r : integer;
  begin
    assert child'path_name = ":process_local_names:main:child";
    assert child'instance_name = ":process_local_names(a):main:child";
    report "child " & child'path_name & " | " & child'instance_name;
    report "k " & k'path_name & " | " & k'instance_name;
    report "main " & main'path_name & " | " & main'instance_name;
    pr(1);
    r := fn(1);
    r := twice(r, true);
    wait;
  end process;
  process
    variable u : integer;
  begin
    wait for 1 ns;
    assert u'path_name = ":process_local_names::u";
    report "u " & u'path_name & " | " & u'instance_name;
    wait;
  end process;
  b : block
  begin
    q : process
      variable v : integer;
    begin
      wait for 2 ns;
      report "v " & v'path_name & " | " & v'instance_name;
      wait;
    end process;
    process
      variable w : integer;
    begin
      wait for 3 ns;
      report "w " & w'path_name & " | " & w'instance_name;
      wait;
    end process;
  end block;
  g : for i in 0 to 1 generate
    gp : process
      variable v : integer;
    begin
      wait for (4 + i) * 1 ns;
      report "gv " & v'path_name & " | " & v'instance_name;
      wait;
    end process;
  end generate;
end architecture;
