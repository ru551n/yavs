-- difftest: top=access_file std=2019
-- Implicit DEALLOCATE (LRM 2019 5.4.3) and the 2019 file operations FILE_CANSEEK,
-- FILE_STATE, FILE_MODE, FILE_SIZE (LRM 2019 5.5.2) end to end (#163).
use std.textio.all;
entity access_file is end entity;

architecture test of access_file is
  type int_ptr is access integer;
begin
  process
    variable p : int_ptr := new integer'(5);
    file f : text;
    variable l : line;
  begin
    p.all := p.all + 1;
    report "p.all = " & integer'image(p.all);
    deallocate(p);
    assert p = null severity failure;
    assert file_state(f) = state_closed severity failure;
    file_open(f, "access_file.txt", write_mode);
    write(l, string'("hello"));
    writeline(f, l);
    assert file_canseek(f) and file_mode(f) = write_mode severity failure;
    file_close(f);
    file_open(f, "access_file.txt", read_mode);
    report "size " & integer'image(file_size(f)) & ", canseek "
      & boolean'image(file_canseek(f));
    file_close(f);
    assert not file_canseek(output) severity failure;
    wait;
  end process;
end architecture;
