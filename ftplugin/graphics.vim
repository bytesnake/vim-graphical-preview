if exists("g:loaded_graphical_preview")
    echo "Exists"
    finish
endif
let g:loaded_graphical_preview = 1

let s:path = resolve(expand('<sfile>:p:h') . "/../")
let s:inst = libcallex#load(s:path . "/target/release/libvim_graphical_preview.so")
let s:folds = []

function! PrintError(msg) abort
    execute 'normal! \<Esc>'
    echohl ErrorMsg
    echomsg a:msg
    echohl None
endfunction

function! DrawInner(id)
    let res = s:inst.call("draw", [""], "string")
    let res = json_decode(res)

    if has_key(res, 'err')
	call PrintError("Error: " . res['err'])
    elseif has_key(res, 'ok') && res['ok'] == 1
	call Draw()
    endif
endfunction

function! Draw()
    if exists("g:timer")
        call timer_stop(g:timer)
    endif

    let g:timer = timer_start(50, "DrawInner")
endfunction

function! s:UpdateMetadata()
    let winpos = win_screenpos("0")
    if exists('&number')
        let winpos[1] += &numberwidth
    endif

    let metadata = {
       \'file_range': [line("w0"), line("w$") - &cmdheight + 1],
       \'viewport': [&lines - &cmdheight - 1, &columns],
       \'cursor': getcurpos()[1],
       \'winpos': winpos,
       \'char_height': 0,
       \}

    call s:inst.call("update_metadata", [json_encode(metadata)], "")
    call Draw()
endfunction

function! s:UpdateFolds()
    call s:UpdateMetadata()
    let l:folding_state = []
    for lnum in s:folds
        call add(l:folding_state, [lnum, foldclosedend(lnum)])
    endfor
    mode
    let any_changed =  s:inst.call("set_folds", [json_encode(folding_state)], "")
    if any_changed
        call Draw()
    endif
endfunction

function! s:TextChanged()
    call s:UpdateMetadata()
    let current_buf = join(getline(1,'$'), "\n")
    let res = s:inst.call("update_content", [current_buf], "string")
    let res = json_decode(res)['ok']
    if has_key(res, 'update_folding')
        let s:folds = res['update_folding']
        call s:UpdateFolds()
    endif
    if res['should_redraw']
        call Draw()
    endif
endfunction

function! s:ClearAll()
    call s:inst.call("clear_all", [""], "")
    mode
endfunction

:autocmd VimEnter,TextChanged,InsertLeave * call <SID>TextChanged()
:autocmd VimResized * call <SID>UpdateMetadata()
:autocmd CursorMoved * call <SID>UpdateMetadata()
:autocmd InsertEnter * call <SID>ClearAll()

map zo :foldopen<CR>:call <SID>UpdateFolds()<CR>
map zc :foldclose<CR>:call <SID>UpdateFolds()<CR>
map zO :foldopen!<CR>:call <SID>UpdateFolds()<CR>
map zC :foldclose!<CR>:call <SID>UpdateFolds()<CR>
