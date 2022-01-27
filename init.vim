source libcallex-vim/autoload/libcallex.vim

let g:inst = libcallex#load("target/release/libvim_math.so")

function! DrawInner()
    let again = g:inst.call("draw", [""], "number")

    if again == 1
        call Draw()
    endif
endfunction

function! Draw()
    if exists("g:timer")
        call timer_stop(g:timer)
    endif

    let g:timer = timer_start(200, { tid -> execute('call DrawInner()')})
endfunction

function! UpdateMetadata()
    let metadata = {'start': line("w0"), 'end': line("w$") - &cmdheight + 1, 'cursor': getcurpos()[1], 'height': &lines - &cmdheight - 1, 'width': &columns}
    call g:inst.call("update_metadata", [json_encode(metadata)], "")
    call Draw()
endfunction

function! TextChanged()
    call UpdateMetadata()
    let current_buf = join(getline(1,'$'), "\n")
    if g:inst.call("update_content", [current_buf], "")
        call Draw()
    endif
endfunction

function! ClearAll()
    call g:inst.call("clear_all", [""], "")
    mode
endfunction

:autocmd TextChanged,InsertLeave * call TextChanged()
:autocmd VimResized * call UpdateMetadata()
:autocmd CursorMoved * call UpdateMetadata()
:autocmd InsertEnter * call ClearAll()

:call TextChanged()
