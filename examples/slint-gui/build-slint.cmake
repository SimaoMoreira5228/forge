execute_process(
    COMMAND ${CMAKE_COMMAND}
        -S vendor/slint/api/cpp
        -B forge-out/slint-build
        -DBUILD_SHARED_LIBS=OFF
        -DSLINT_FEATURE_COMPILER=OFF
        -DCMAKE_BUILD_TYPE=Release
    RESULT_VARIABLE configure_result)
if(configure_result)
    message(FATAL_ERROR "slint configure failed: ${configure_result}")
endif()
execute_process(
    COMMAND ${CMAKE_COMMAND}
        --build forge-out/slint-build
        --target cargo-build_slint_cpp
        --parallel
    RESULT_VARIABLE build_result)
if(build_result)
    message(FATAL_ERROR "slint build failed: ${build_result}")
endif()
