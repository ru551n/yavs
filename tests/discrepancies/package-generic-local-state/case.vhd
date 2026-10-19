-- (from the differential corpus)
-- ref=hand: the reference simulator crashes during elaboration (a package
-- instance with a shared variable in an architecture instantiated three times);
-- the third reference simulator agrees with the expected output.
-- Generic package instances with state declared in an architecture: one set of
-- objects per entity instance.
package acc_g is
  generic (base : integer);
  type acc_t is protected
    procedure add(x : integer);
    impure function get return integer;
  end protected;
  procedure add(x : integer);
  impure function get return integer;
  signal sig : integer := base;
end package;
package body acc_g is
  type acc_t is protected body
    variable total : integer := base;
    procedure add(x : integer) is
    begin
      total := total + x;
    end procedure;
    impure function get return integer is
    begin
      return total;
    end function;
  end protected body;
  shared variable acc : acc_t;
  procedure add(x : integer) is
  begin
    acc.add(x);
  end procedure;
  impure function get return integer is
  begin
    return acc.get;
  end function;
end package body;

entity unit is
  generic (id : integer);
end entity;
architecture a of unit is
  package mine is new work.acc_g generic map (base => id * 100);
begin
  drv : process
  begin
    mine.sig <= mine.sig + id;
    wait for id * 1 ns;
    mine.add(id);
    mine.add(id);
    report "unit " & integer'image(id) & " acc=" & integer'image(mine.get)
      & " sig=" & integer'image(mine.sig);
    wait;
  end process;
end architecture;

entity pkg_generic_local_state is end entity;
architecture t of pkg_generic_local_state is
begin
  g : for i in 1 to 3 generate
    u : entity work.unit generic map (id => i);
  end generate;

end architecture;
