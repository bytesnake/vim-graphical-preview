source libcallex-vim/autoload/libcallex.vim

let g:inst = libcallex#load("target/release/libvim_math.so")

function! UpdateMetadata()
    let metadata = {'start': line("w0"), 'end': line("w$"), 'cursor': getcurpos()[1], 'height': &lines - &cmdheight, 'width': &columns}
    ":mode
    :call g:inst.call("update_metadata", [json_encode(metadata)], "")
endfunction

function! TextChanged()
    :call UpdateMetadata()
    let current_buf = join(getline(1,'$'), "\n")
    :mode
    :call g:inst.call("update_content", [current_buf], "")
endfunction

function! ClearAll()
    ":call g:inst.call("clear_all", [""], "")
    :mode
endfunction

:autocmd TextChanged,InsertLeave * :call TextChanged()
:autocmd CursorMoved,VimResized * :call UpdateMetadata()
:autocmd InsertEnter * :call ClearAll()

":autocmd InsertEnter * :call Draw()
