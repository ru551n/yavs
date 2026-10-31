-- difftest: top=generic_file_package_arch std=2019 ref=hand
-- A package instance declared in an architecture whose generic package declares a file object
-- (#925). ref=hand: sim1 has no file operations for a generic file type; the value read back is
-- what was written.
package gfa_p is
  generic (type designated_subtype; type file_type is file of designated_subtype; name : string);
  file f : file_type;
  procedure put(v : designated_subtype);
  procedure get(v : out designated_subtype);
end package;
package body gfa_p is
  procedure put(v : designated_subtype) is begin
    file_open(f, name, write_mode);
    write(f, v);
    file_close(f);
  end procedure;
  procedure get(v : out designated_subtype) is begin
    file_open(f, name, read_mode);
    read(f, v);
    file_close(f);
  end procedure;
end package body;
entity generic_file_package_arch is end;
architecture a of generic_file_package_arch is
  type ft is file of integer;
  package q is new work.gfa_p
    generic map (designated_subtype => integer, file_type => ft, name => "generic_file_package_arch.bin");
begin
  process
    variable n : integer;
  begin
    q.put(42);
    q.get(n);
    report integer'image(n);
    assert n = 42 severity failure;
    wait;
  end process;
end architecture;
